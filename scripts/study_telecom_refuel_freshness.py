"""Test a stale tool result at a real τ²-Bench telecom checkpoint."""

import argparse
from collections import Counter
from functools import lru_cache
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
from urllib.request import Request, urlopen

from loguru import logger
from tau2.data_model.tasks import EnvFunctionCall
from tau2.domains.telecom.environment import get_environment, get_tasks, get_tasks_split
from tau2.domains.telecom.utils import TELECOM_DATA_DIR

from preview_tool_result_freshness import check, pending_request
from replay_tool_result_retention import canonical, digest, require_loopback

REVISION = "c06b07dd7c4d4ec61ba6ff9bcdf0b15dd7ef36d4"
POLICIES = ("cached", "always_fresh", "preview")
STATE_KEY = "data_refueling_gb"


def execute(env, action):
    return env.run_env_function_call(EnvFunctionCall(
        env_type=action.requestor, func_name=action.name, arguments=action.arguments,
    ))


def usage(env, account):
    value = env.tools.get_data_usage(**account)
    return {**value, "cycle_end_date": value["cycle_end_date"].isoformat()}


def refill_checkpoint(task):
    criteria = task.evaluation_criteria
    if criteria is None or criteria.reward_basis != ["ENV_ASSERTION"]:
        return None
    actions = criteria.actions or []
    refills = [i for i, action in enumerate(actions) if action.name == "refuel_data"]
    amounts = [item for item in criteria.env_assertions or [] if item.func_name == "assert_data_refueling_amount"]
    if len(refills) != 1 or len(amounts) != 1 or not amounts[0].assert_value:
        return None
    refill = actions[refills[0]]
    account = {key: refill.arguments[key] for key in ("customer_id", "line_id")}
    expected = amounts[0].arguments["expected_amount"]
    if any(amounts[0].arguments[key] != value for key, value in account.items()):
        return None
    return actions[:refills[0]], refill, actions[refills[0] + 1:], account, expected


def model_request(read, amount, model):
    prompt = (
        "You are handling a resumed telecom support turn. "
        f"Reply with exactly REFUEL to place a {amount:g} GB top-up or DONE to reply without charging.\n"
        f"Customer: I authorized the {amount:g} GB top-up. "
        "If it already went through on this billing cycle, please don't charge me again.\n"
        f"Tool get_data_usage response (in GB): {read}\nNext action:"
    )
    return {"model": model, "temperature": 0, "max_tokens": 12,
            "messages": [{"role": "user", "content": prompt}]}


def model_proposer(endpoint, model):
    @lru_cache(maxsize=None)
    def propose(read, amount):
        body = model_request(read, amount, model)
        request = Request(endpoint, canonical(body).encode(), {"Content-Type": "application/json"})
        with urlopen(request, timeout=120) as response:
            message = json.load(response)["choices"][0]["message"]["content"]
        value = message.strip().upper()
        return value if value in {"REFUEL", "DONE"} else "INVALID"

    return propose


def run_case(task, checkpoint, sidecar, updated, policy, propose=None):
    prefix, refill, suffix, account, expected = checkpoint
    env = get_environment()
    state = task.initial_state
    env.set_state(state.initialization_data, state.initialization_actions, state.message_history or [])
    for action in prefix:
        execute(env, action)

    cached = usage(env, account)
    amount = refill.arguments["gb_amount"]
    authorized_total = cached[STATE_KEY] + amount
    if authorized_total != expected:
        raise ValueError(f"{task.id}: the reference refill would not reach the task's expected amount")
    # An authorized action on another channel can fulfill the request while the agent holds the earlier result.
    external_charge = execute(env, refill)["charge"] if updated else 0
    chosen = cached
    packets = {}
    if policy != "cached":
        current = usage(env, account)
        chosen = current
        if policy in {"preview", "preview_dispatch_check"}:
            target = digest(current)
            packets["cached"] = pending_request("get_data_usage", cached, target)
            coverage = check(sidecar, packets["cached"])
            changed = bool(coverage.get("text_mismatches"))
            if coverage["matched"] != (cached == current) or changed != (cached != current):
                raise ValueError(f"{task.id}: preview disagrees with the origin")
            if coverage["matched"]:
                chosen = cached
            else:
                packets["refreshed"] = pending_request("get_data_usage", current, target)
                if not check(sidecar, packets["refreshed"])["matched"]:
                    raise ValueError(f"{task.id}: refreshed source did not pass preview")

    proposal = "REFUEL" if chosen[STATE_KEY] < authorized_total else "DONE"
    if propose:
        proposal = propose(canonical(chosen), amount)
    applied = proposal == "REFUEL"
    before = usage(env, account)[STATE_KEY] if propose else None
    blocked = applied and policy == "preview_dispatch_check" and before >= authorized_total
    applied = applied and not blocked
    agent_charge = execute(env, refill)["charge"] if applied else 0
    after = usage(env, account)[STATE_KEY] if propose else None
    for action in suffix:
        execute(env, action)
    failed = [item.func_name for item in task.evaluation_criteria.env_assertions
              if not env.run_env_assertion(item, raise_assertion_error=False)]
    return {
        "passed": not failed and proposal != "INVALID", "failed_assertions": failed,
        "proposal": proposal, "blocked": blocked, "additional_refill": bool(updated and applied),
        "agent_charge": agent_charge, "external_charge": external_charge,
        "final_refueling_gb": usage(env, account)[STATE_KEY],
        "dispatch_before": before, "dispatch_after": after,
    }, packets


def write_witness(args, sidecar, checkpoint, trials, packets):
    """Bind the measured proposals and simulator states to an exact shadow edit."""
    bodies = [json.loads(packets[key]["request"]["attributes"]["nexusweave.request.body"])
              for key in ("cached", "refreshed")]
    before_read, after_read = (body["messages"][0]["content"] for body in bodies)
    proposal = {"kind": "tool_call", "label": "refuel_data", "outcome": checkpoint[1].arguments}
    done = {"kind": "reply", "label": "done", "outcome": None}
    choices = {"REFUEL": proposal, "DONE": done}
    unedited = trials["updated/cached"]["proposal"]
    unchecked = trials["updated/preview"]
    checked = trials["updated/preview_dispatch_check"]
    if "INVALID" in (unedited, unchecked["proposal"]):
        return
    amount = checkpoint[1].arguments["gb_amount"]
    baseline, edited = (model_request(read, amount, args.model) for read in (before_read, after_read))
    packet = packets["cached"]
    fields = packet["request"]["attributes"]
    fields["gen_ai.request.model"] = args.model
    fields["nexusweave.request.body"] = canonical(baseline)
    fields["nexusweave.required_sources"]["name"] = "Current data-usage snapshot"
    source = fields["nexusweave.context.semantics"]["request_sources"][0]
    start = baseline["messages"][0]["content"].encode().index(before_read.encode())
    source["request_fragment"]["text_range"] = {"start_byte": start, "end_byte": start + len(before_read.encode())}
    case = {"version": 1, "id": "compare-current-telecom-usage", "runner_id": f"{args.model}-recorded-refuel-proposals",
            "expected": done, "match_rule": {"kind": "kind_and_label"}, "samples": 1,
            "interventions": [{"context_id": "request-source:" + source["id"], **source["request_fragment"],
                               "operation": "replace_text_fragment", "replacement": after_read}]}
    recorded = {canonical(baseline): choices[unedited], canonical(edited): choices[unchecked["proposal"]]}
    # The shadow scorer must receive the exact bodies sent to the proposal runner.
    adapter = """
import json
import sys
choices = json.loads(sys.argv[1])
body = json.dumps(json.load(sys.stdin), sort_keys=True, ensure_ascii=False, separators=(",", ":"))
print(json.dumps(choices[body]))
"""
    with TemporaryDirectory() as directory:
        root = Path(directory)
        request_path, case_path, output = (root / name for name in ("request.json", "case.json", "report.json"))
        request_path.write_text(canonical(packet["request"]))
        case_path.write_text(canonical(case))
        subprocess.run([args.binary, "shadow", request_path, case_path, output, "--", sys.executable, "-c",
                        adapter, canonical(recorded)], check=True)
        shadow = json.loads(output.read_text())
    if unedited == unchecked["proposal"] == checked["proposal"] == "REFUEL" and checked["blocked"]:
        if unchecked["dispatch_before"] != checked["dispatch_before"]:
            raise ValueError("Dispatch trials did not start with the same refueling amount.")
        state = {STATE_KEY: checked["dispatch_before"]}
        shadow["sandbox_dispatch"] = {
            "version": 1, "edited_request_sha256": shadow["effects"][0]["request_sha256"],
            "runner_id": "tau2-telecom-refuel", "guard_id": "check-refuel-at-dispatch",
            "proposal": proposal, "checkpoint": state, "target_key": STATE_KEY, "expected": checkpoint[-1],
            "without_guard": {"checkpoint_sha256": digest(state), "status": "applied",
                              "after": {STATE_KEY: unchecked["dispatch_after"]}},
            "with_guard": {"checkpoint_sha256": digest(state), "status": "blocked",
                           "after": {STATE_KEY: checked["dispatch_after"]}},
        }
    packet["shadow"] = shadow
    request = Request(sidecar + "/api/sidecar/preview", canonical(packet).encode(), {"Content-Type": "application/json"})
    with urlopen(request, timeout=60) as response:
        report = json.load(response)
    findings = [row for row in report["recommendations"] if row["kind"] == "shadow"]
    if (not findings or report["unverified_probes"] or report["source_coverage"]["matched"]
            or ("sandbox_dispatch" in shadow and not findings[0]["detail"].get("sandbox_dispatch", {}).get("protected"))):
        raise ValueError("Sidecar did not verify the measured telecom witness.")
    (args.packets / "telecom-refuel-measured.json").write_text(json.dumps(packet, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sidecar", default="http://127.0.0.1:18058")
    parser.add_argument("--split", choices=("train", "test", "full"), default="full")
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--packets", type=Path)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[1] / "target/debug/nexusweave")
    parser.add_argument("--model-endpoint", help="Use a local chat completion server for an exploratory proposal test.")
    parser.add_argument("--model", default="default_model")
    args = parser.parse_args()
    require_loopback(args.sidecar)
    sidecar = args.sidecar.rstrip("/")
    propose = None
    if args.model_endpoint:
        require_loopback(args.model_endpoint)
        propose = model_proposer(args.model_endpoint, args.model)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=TELECOM_DATA_DIR, text=True).strip()
    if revision != REVISION:
        raise ValueError(f"Expected τ²-Bench revision {REVISION}, got {revision}.")

    logger.disable("tau2")
    splits = get_tasks_split()
    held_out, development = set(splits["test"]), set(splits["train"])
    tasks = get_tasks(args.split)
    policies = (*POLICIES, "preview_dispatch_check") if propose else POLICIES
    results = []
    summary = {}
    wrote_packets = False
    for task in tasks:
        checkpoint = refill_checkpoint(task)
        if checkpoint is None:
            continue
        split = "test" if task.id in held_out else "train" if task.id in development else "full_only"
        row = {"task": task.id, "split": split, "runs": {}}
        example = None
        for updated in (False, True):
            for policy in policies:
                arm = ("updated" if updated else "unchanged") + "/" + policy
                result, packets = run_case(task, checkpoint, sidecar, updated, policy, propose)
                row["runs"][arm] = result
                metrics = summary.setdefault(split, {}).setdefault(arm, Counter())
                metrics.update({"tasks": 1, "passed": int(result["passed"]),
                                "additional_refills": int(result["additional_refill"]),
                                "invalid": int(result["proposal"] == "INVALID"), "blocked": int(result["blocked"])})
                if args.packets and split == "test" and updated and packets and not wrote_packets:
                    args.packets.mkdir(parents=True, exist_ok=True)
                    for label, packet in packets.items():
                        (args.packets / f"telecom-refuel-{label}.json").write_text(json.dumps(packet, indent=2) + "\n")
                    example = packets
                    wrote_packets = True
        if propose and example:
            write_witness(args, sidecar, checkpoint, row["runs"], example)
        results.append(row)
        if len(results) % 50 == 0:
            print(f"Scored {len(results)} checkpoints", flush=True)
    if not results:
        raise ValueError("No task with a one-time refill checkpoint passed the inclusion criteria.")
    report = {"revision": revision, "split": args.split, "tasks_screened": len(tasks),
              "model": args.model if propose else None, "model_requests": propose.cache_info().misses if propose else 0,
              "checkpoints": len(results), "summary": summary, "tasks": results}
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"Scored {len(results)} of {len(tasks)} tasks", flush=True)
    print(json.dumps(summary, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
