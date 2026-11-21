"""Compare stale-input and operation-level controls in the public AppWorld simulator."""

import argparse
from collections import Counter
from decimal import Decimal
from importlib.metadata import version
import json
import os
from pathlib import Path
import re
from typing import NamedTuple

from appworld import AppWorld, load_task_ids

from preview_tool_result_freshness import SOURCE, check, pending_request
from replay_tool_result_retention import canonical, digest, require_loopback

APPWORLD_VERSION = "0.1.3.post1"
SPLITS = ("train", "dev", "test_normal", "test_challenge")
HANDOFF = "Operations handoff: Verify the revised schedule before publishing."
CONCURRENT_EDIT = "Schedule update: Review moved to Thursday at 10:00."


class Trial(NamedTuple):
    metrics: dict
    packets: dict


class IneligibleTrial(Exception):
    """A known simulator state prevents this controlled comparison."""


def auth(world, app, passwords):
    token = world.apis[app].login(username=world.task.supervisor.email, password=passwords[app])
    if "access_token" not in token:
        raise IneligibleTrial("simulator_login_unavailable")
    return {"access_token": token["access_token"]}


def pending_payment_requests(api, token):
    try:
        result = api.show_received_payment_requests(status="pending", page_limit=20, **token)
    except Exception as error:
        if not re.fullmatch(r"User with ID \d+ not found\.", str(error)):
            raise
        raise IneligibleTrial("payment_list_references_missing_user") from None
    if not isinstance(result, list):
        raise ValueError("Expected the simulator's pending payment list.")
    return result


def payment_trial(world, passwords, sidecar):
    api = world.apis.venmo
    token = auth(world, "venmo", passwords)
    payer = world.models.venmo.User.find_one_or_raise(email=world.task.supervisor.email)
    balance = Decimal(str(payer.venmo_balance))
    pending = pending_payment_requests(api, token)
    selected = next((row for row in pending if Decimal(str(row["amount"])) <= balance), None)
    if selected is None:
        raise IneligibleTrial("no_fundable_pending_payment")
    target = selected["payment_request_id"]
    transactions = world.models.venmo.Transaction
    before_ids = {tx.id for tx in transactions.find_all(sender_id=payer.id)}

    # This same write is the valid-action control from the initial state and the external write for the stale retry.
    first = api.approve_payment_request(payment_request_id=target, **token)
    after_first = {tx.id for tx in transactions.find_all(sender_id=payer.id)}
    fresh = world.models.venmo.PaymentRequest.by_id_or_raise(target)
    if (first.get("message") != "Payment request approved." or len(after_first - before_ids) != 1
            or not before_ids <= after_first or not fresh.approved_at):
        raise ValueError("Expected exactly one transaction for the original simulator payment.")
    before_retry = world.models.venmo.User.by_id_or_raise(payer.id).venmo_balance

    pending_now = pending_payment_requests(api, token)
    if any(row["payment_request_id"] == target for row in pending_now):
        raise ValueError("An approved simulator payment is still in the pending list.")
    previous = {"payment_request_id": target, "status": "pending"}
    current = {"payment_request_id": target, "status": "approved"}
    packet = pending_request("Payment request status", previous, digest(current))
    coverage = check(sidecar, packet)
    retry = api.approve_payment_request(payment_request_id=target, **token)
    after_retry = {tx.id for tx in transactions.find_all(sender_id=payer.id)}
    balance_after_retry = world.models.venmo.User.by_id_or_raise(payer.id).venmo_balance
    # Refreshing a list and selecting a new item by position can change the authorized target.
    available_balance = Decimal(str(balance_after_retry))
    alternative = next((row for row in pending_now if Decimal(str(row["amount"])) <= available_balance), None)
    retargeted = None
    if alternative is not None:
        other_id = alternative["payment_request_id"]
        response = api.approve_payment_request(payment_request_id=other_id, **token)
        final_ids = {tx.id for tx in transactions.find_all(sender_id=payer.id)}
        retargeted = (response.get("message") == "Payment request approved."
                      and len(final_ids - after_retry) == 1
                      and bool(world.models.venmo.PaymentRequest.by_id_or_raise(other_id).approved_at))
    return Trial({
        "valid_approval_applied": True,
        "preview_detected_stale": coverage.get("text_mismatches") == [SOURCE],
        "same_id_retry_rejected": retry.get("message") == "This payment request has already been completed.",
        "same_id_retry_preserved_state": after_first == after_retry and before_retry == balance_after_retry,
        "pending_list_has_another_id": bool(pending_now),
        "refresh_by_position_wrote_other_id": retargeted,
    }, {"payment-stale": packet})


def note_projection(note):
    return {name: note[name] for name in ("note_id", "content", "pinned")}


def content_packet(captured, current):
    packet = pending_request("Shared note text", captured, digest(current["content"]))
    fields = packet["request"]["attributes"]
    fragment = fields["nexusweave.context.semantics"]["request_sources"][0]["request_fragment"]
    # Canonical JSON puts the content key first. Anchor only that encoded string in the tool message.
    prefix = len('{"content":'.encode())
    fragment["text_range"] = {"start_byte": prefix, "end_byte": prefix + len(canonical(captured["content"]).encode())}
    return packet


def note_trial(world, passwords, sidecar):
    api = world.apis.simple_note
    token = auth(world, "simple_note", passwords)
    notes = api.search_notes(page_limit=5, **token)
    if not isinstance(notes, list):
        raise ValueError("Expected the simulator's note list.")
    if not notes:
        raise IneligibleTrial("no_notes")
    key = {"note_id": notes[0]["note_id"], **token}
    original = note_projection(api.show_note(**key))
    base = original["content"]
    if len(base.encode()) > 16384:
        raise IneligibleTrial("note_over_preview_budget")
    summary, packets = {}, {}

    for event in ("unchanged", "metadata", "content"):
        pinned = not original["pinned"] if event == "metadata" else original["pinned"]
        origin_text = base + "\n" + CONCURRENT_EDIT if event == "content" else base
        api.update_note(content=origin_text, pinned=pinned, **key)
        current = note_projection(api.show_note(**key))
        if current != {**original, "content": origin_text, "pinned": pinned}:
            raise ValueError("Simulator failed to create the intended external note edit.")
        whole = pending_request("Shared note snapshot", original, digest(current))
        focused = content_packet(original, current)
        whole_refresh = not check(sidecar, whole)["matched"]
        content_refresh = not check(sidecar, focused)["matched"]
        methods = {
            "cached_replace": base,
            "refresh_on_snapshot_mismatch": current["content"] if whole_refresh else base,
            "refresh_on_content_mismatch": current["content"] if content_refresh else base,
        }
        expected = {**current, "content": origin_text + "\n" + HANDOFF}
        passed = {}
        for name, body in methods.items():
            api.update_note(content=body + "\n" + HANDOFF, **key)
            passed[name] = note_projection(api.show_note(**key)) == expected
        api.update_note(content=origin_text, **key)
        api.add_content_to_note(append_or_prepend="append", added_content=HANDOFF, **key)
        passed["native_append"] = note_projection(api.show_note(**key)) == expected
        summary[event] = {"snapshot_refresh": whole_refresh, "content_refresh": content_refresh, "passed": passed}
        if event != "unchanged":
            packets[f"note-{event}-snapshot"] = whole
            packets[f"note-{event}-content"] = focused
    return Trial(summary, packets)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sidecar", default="http://127.0.0.1:18062")
    parser.add_argument("--splits", choices=SPLITS, nargs="+", default=SPLITS)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--packets", type=Path)
    parser.add_argument("--limit", type=int)
    args = parser.parse_args()
    require_loopback(args.sidecar)
    if version("appworld") != APPWORLD_VERSION or not os.environ.get("APPWORLD_ROOT"):
        parser.error(f"Set APPWORLD_ROOT to the local dataset and install appworld=={APPWORLD_VERSION}.")
    results, saved = [], set()
    sidecar = args.sidecar.rstrip("/")
    manifest, screened = {}, Counter()
    for split in args.splits:
        ids = load_task_ids(split)
        manifest[split] = digest(ids)
        for task in ids[:args.limit]:
            with AppWorld(task_id=task, experiment_name="nexusweave-tool-state", load_ground_truth=False,
                          remote_apis_url=None, remote_environment_url=None, raise_on_failure=False,
                          parse_datetimes=False, munchify_response=False) as world:
                passwords = {row["account_name"]: row["password"] for row in world.apis.supervisor.show_account_passwords()}
                row = {"split": split, "task": task, "account": digest(world.task.supervisor.email)[:16], "excluded": {}}
                for name, run in (("venmo", payment_trial), ("simple_note", note_trial)):
                    row[name] = None
                    if name not in passwords:
                        row["excluded"][name] = "no_app_account"
                        continue
                    try:
                        trial = run(world, passwords, sidecar)
                    except IneligibleTrial as error:
                        row["excluded"][name] = str(error)
                        continue
                    row[name] = trial.metrics
                    if args.packets and name not in saved and split.startswith("test"):
                        args.packets.mkdir(parents=True, exist_ok=True)
                        for label, packet in trial.packets.items():
                            (args.packets / f"appworld-{label}.json").write_text(json.dumps(packet, indent=2) + "\n")
                        saved.add(name)
            results.append(row)
            screened[split] += 1
            if len(results) % 30 == 0:
                print(f"Checked {len(results)} task snapshots", flush=True)
    report = {"appworld_version": APPWORLD_VERSION, "split_task_ids_sha256": manifest,
              "screened": screened, "tasks": results}
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print("Screened", dict(screened), flush=True)


if __name__ == "__main__":
    main()
