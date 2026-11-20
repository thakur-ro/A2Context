"""Time scoped routing card and factor queries over 256 captured calls."""
import argparse
import json
from pathlib import Path
from statistics import median
import sys
from tempfile import TemporaryDirectory
from time import perf_counter

from prefill_cohort import ATTRIBUTE, export
from replicate_routing import RESULTS, SCRIPTS

sys.path.insert(0, str(SCRIPTS))
from check_sidecar import get, send_traces, server


def benchmark(binary, folder, sampled=False, edited_sources=False):
    records = json.loads((RESULTS.with_name("sampled-results.json") if sampled else RESULTS).read_text())
    org = records[0]["extra"]["metadata"]["organization_id"]
    distinct = len({r["extra"]["metadata"]["nexusweave.request.body"] for r in records})
    document = export(records, {r["id"]: r["extra"]["metadata"][ATTRIBUTE] for r in records})
    spans = document["resourceSpans"][0]["scopeSpans"][0]["spans"]
    with server(binary, folder / "store", "--capture-content") as base:
        for batch in range(64):
            for i, span in enumerate(spans):
                identifier = batch * len(spans) + i + 1
                span.update(traceId=f"{identifier:032x}", spanId=f"{identifier:016x}")
                thread = next(a for a in span["attributes"] if a["key"] == "gen_ai.conversation.id")
                thread["value"]["stringValue"] = f"conversation-{identifier}"
            send_traces(base, document)
        timings = []
        factor_timings = []
        edit_timings = []
        for i in range(21):
            start = perf_counter()
            card, = get(base, "findings", org=org, kind="prefill")["rows"]
            elapsed = (perf_counter() - start) * 1000
            assert card["threads"] == card["traces"] == 256
            if "routing_profiles" in card:
                assert card["routing_conversations"] == 256 and sum(p["inputs"] for p in card["routing_profiles"]) == distinct
            start = perf_counter()
            factor, = get(base, "factors", org=org)["rows"]
            factor_elapsed = (perf_counter() - start) * 1000
            assert factor["requests"] == 256
            if edited_sources:
                query = dict(org=org, level=1, by="edit")
                start = perf_counter()
                edit, = get(base, "outcome-factors", **query)["rows"]
                members = get(base, "conversation-outcomes", **query, cohort_factor=edit["factor"])
                pair_ms = (perf_counter() - start) * 1000
                assert [s["source"] for s in edit["sources"]] == ["request-type"]
                assert edit["conversations"] == edit["cohort_size"] and len(members["rows"]) == 15
                if i:
                    edit_timings.append(pair_ms)
            if i:
                factor_timings.append(factor_elapsed)
                timings.append(elapsed)
        result = {"requests": 256, "distinct_inputs": distinct, "timed_reads": len(timings),
                "median_ms": round(median(timings), 2), "p95_ms": round(sorted(timings)[18], 2),
                "factor_median_ms": round(median(factor_timings), 2), "factor_p95_ms": round(sorted(factor_timings)[18], 2),
                "queue_drops": get(base, "health")["queue_drops"]}
        if edited_sources:
            result.update(edited_conversations=edit["conversations"], edit_pair_median_ms=round(median(edit_timings), 2),
                          edit_pair_p95_ms=round(sorted(edit_timings)[18], 2))
        return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--sampled", action="store_true", help="Use four sampled repeats of one input.")
    parser.add_argument("--edited-sources", action="store_true", help="Also time the changed-edit group and its first page of conversations.")
    args = parser.parse_args()
    with TemporaryDirectory() as folder:
        print(json.dumps(benchmark(args.binary.resolve(), Path(folder), args.sampled, args.edited_sources)))
