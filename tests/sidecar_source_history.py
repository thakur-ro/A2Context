"""Follow one source across retained observations, including calls outside the first card."""
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from check_sidecar import case_span, export, get, send_traces, server


def verify(binary):
    with TemporaryDirectory() as tmp, server(binary, Path(tmp) / "history.sqlite", "--capture-content") as base:
        spans = []
        for i in range(256):
            span = case_span("returns", "conflict", i, shift=-i*86400+i)
            attrs = {item["key"]:item["value"] for item in span["attributes"]}
            # Identical display names from different users represent different conversations.
            attrs["gen_ai.conversation.id"]["stringValue"] = "return-" + str(i // 4)
            attrs["nexusweave.user.id"]["stringValue"] = "sam" if i % 2 else "jo"
            if i >= 252:
                span["attributes"] = [item for item in span["attributes"] if item["key"] != "gen_ai.conversation.id"]
            spans.append(span)
        # Two calls in the same trace count separately and open the right span.
        extra = {**spans[0], "spanId":"3333333333333333"}
        spans.append(extra)
        for start in range(0, len(spans), 32):
            send_traces(base, export(*spans[start:start + 32]))
        send_traces(base, export(extra))
        card, = get(base, "findings")["rows"]
        evidence = card["evidence"]
        source = evidence["detail"]["sources"][0]
        key = json.dumps([evidence["producer"],evidence["organization"],evidence["workflow"],
                          source["source"],source["version"],source["role"]])
        def history(**filters):
            return get(base, "source-history", source_key=key, **filters)
        page = history()
        assert len(page["rows"]) == 15 and page["next_offset"] == 15
        rows = page["rows"][:]
        while page["next_offset"] is not None:
            page = history(offset=page["next_offset"])
            rows.extend(page["rows"])
        assert len(rows) == 130
        assert {(row["total_threads"],row["total_calls"],row["unthreaded_calls"]) for row in rows} == {(126,257,4)}
        assert sum(row["calls"] for row in rows) == 257
        jo, sam = sorted((r for r in rows if r["thread"] == "return-0"), key=lambda r:r["user"])
        assert (jo["calls"],sam["calls"]) == (3,2)
        calls = history(thread=jo["thread_key"])["rows"]
        assert len({(r["trace"],r["call"]) for r in calls}) == 3
        assert [r["started"] for r in calls] == sorted(r["started"] for r in calls)
        assert history(thread=jo["thread_key"],user="sam")["rows"] == []
        assert history(user="sam")["rows"][0]["total_threads"] == 63
        for row in rows:
            if row["thread_key"] is None:
                call, = history(unthreaded_trace=row["unthreaded_trace"])["rows"]
                assert call["thread"] is None and call["trace"] == row["unthreaded_trace"]
        assert history(after="2026-10-13")["rows"] == []
        print("Verified one grouped card across 257 calls, 126 identified conversations and four unthreaded calls.")


if __name__ == "__main__":
    verify(Path(sys.argv[1]).resolve())
