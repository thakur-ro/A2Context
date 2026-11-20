"""Verify scoped routing recommendations for saved instrumented tool proposals."""

from copy import deepcopy
from pathlib import Path
import sqlite3
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "scripts"), str(ROOT / "examples")]
from check_sidecar import get, send, send_traces, server
from send_tool_masks import samples


def verify(base):
    groups = get(base, "findings", kind="source_mask", workflow="retail-tool-masks")["rows"]
    keyed = {}
    for group in groups:
        mask = group["evidence"]["detail"]
        keyed[tuple(s["source"] for s in mask["sources"]), mask["mask"]["original"]["label"]] = group
    expected = {
        (("stock",), "stock"): {"changed_tool": 3},
        (("stock",), "price"): {"same_proposal": 4},
        (("price",), "stock"): {"same_proposal": 3},
        (("price",), "price"): {"changed_tool": 4},
        (("price", "stock"), "stock"): {"changed_arguments": 3},
        (("price", "stock"), "price"): {"changed_arguments": 3, "token_limit": 1},
        (("product-id",), "stock"): {"changed_arguments": 3},
        (("product-id",), "price"): {"changed_arguments": 4},
    }
    assert {key: group["mask_outcomes"] for key, group in keyed.items()} == expected
    pair = keyed[("price", "stock"), "price"]
    rows = get(base, "occurrences", group=pair["id"], org="retail-north", user="analyst")["rows"]
    assert len(rows) == 2
    limit, = (r for r in rows if r["detail"]["mask"]["outcome"] == "token_limit")
    assert limit["thread"] == "north-1" and limit["detail"]["response_pair"] is None
    assert not get(base, "occurrences", group=pair["id"], org="unrelated")["rows"]
    argument = keyed[("product-id",), "price"]
    rows = get(base, "occurrences", group=argument["id"], org="retail-north")["rows"]
    followup, = (r for r in rows if r["thread"] == "north-1")
    witness = followup["detail"]["mask"]
    assert witness["original"]["outcome_preview"] == '{"product_id":19}'
    assert witness["proposal"]["outcome_preview"] == '{"product_id":42}'
    assert "&continuation=3" in witness["inspector_path"] and witness["trace"] == followup["trace"]


if __name__ == "__main__":
    binary = Path(sys.argv[1]).resolve()
    with TemporaryDirectory() as tmp:
        database = Path(tmp) / "routing.sqlite"
        runs = list(samples())
        with server(binary, database, "--capture-content") as base:
            for export, report in runs:
                send_traces(base, export)
                send(base, "/api/activation-reports", report, expected=201)
                send(base, "/api/activation-reports", report, expected=200)
            verify(base)
            invalid = deepcopy(runs[0][1])
            invalid["id"] += "-invalid-control"
            invalid["continuations"][0]["clean"]["text"] = "Changed output"
            send(base, "/api/activation-reports", invalid, expected=400)
            verify(base)
        with sqlite3.connect(database) as db:
            db.execute("DELETE FROM observations WHERE kind='source_mask'")
            db.execute("PRAGMA user_version=30")
        with server(binary, database, "--capture-content") as base:
            verify(base)
            repeat = deepcopy(runs[0][1])
            repeat["id"] += "-mixed"
            arm = repeat["continuations"][0]
            arm["masked"] = arm["clean"]
            arm["tool_call"] = repeat["baseline_tool_call"]
            send(base, "/api/activation-reports", repeat, expected=201)
            groups = get(base, "findings", kind="source_mask")["rows"]
            counts = next(g["mask_outcomes"] for g in groups if g["mask_outcomes"].get("mixed"))
            assert counts == {"changed_tool":2, "mixed":1}
        with server(binary, Path(tmp) / "redacted.sqlite") as base:
            export, report = runs[0]
            send_traces(base, export)
            send(base, "/api/activation-reports", report, expected=400)
            assert not get(base, "findings", kind="source_mask")["rows"]
    print("Verified routing mask outcomes, replay identity, scope, redaction and migration.")
