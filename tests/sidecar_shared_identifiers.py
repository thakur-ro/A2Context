"""Check cross-conversation evidence after late exports, retries, retention and an upgrade."""

from pathlib import Path
import sqlite3
import sys
from tempfile import TemporaryDirectory

from sidecar_context_claims import span
from check_sidecar import export, get, send_traces, server


def make_call(n, question, **options):
    text = "Before: doc-8\n" + question + "\nFrais — Ex. (sample-42)\nhttps://example.org/url-19"
    reply = "Résumé (sample-42, doc-8, url-19)"
    call = span(n, {"messages": [{"role": "user", "content": text}]}, reply, **options)
    call["endTimeUnixNano"] = str(int(call["startTimeUnixNano"]) + 1000000)
    return call, text, reply


def check(binary, browser=False):
    first, before, _ = make_call(21, "How big?")
    second, _, _ = make_call(22, "What changed?")
    third, after, reply = make_call(23, "Vérifier doc-8?")
    other, _, _ = make_call(24, "Another user's question", user="someone-else")
    flags = ("--capture-content", "--retain-days", "365")
    with TemporaryDirectory() as directory:
        database = Path(directory) / "observations.sqlite"

        def rows():
            return get(base, "findings", kind="shared_identifier", user="usr")["rows"]

        def witness():
            record = get(base, "trace", id="otlp:" + third["traceId"])
            return next(f["detail"] for f in record["findings"] if f["kind"] == "shared_identifier")

        with server(binary, database, *flags) as base:
            send_traces(base, export(third, other))
            assert not rows()
            send_traces(base, export(first))
            (card,) = rows()
            detail = witness()
            assert (
                card["evidence"]["detail"] == detail
                and detail["peer"]["trace"] == "otlp:" + first["traceId"]
            )
            (match,) = detail["matches"]
            assert match["key"] == "sample-42" and match["input_excerpt"] == "Frais — Ex. (sample-42)"
            for field, text in (
                ("input_bytes", after),
                ("peer_bytes", before),
                ("output_bytes", reply),
            ):
                a, b = match[field]
                assert text.encode()[a:b].decode() == match["key"]
            send_traces(base, export(second))
            send_traces(base, export(second))
            assert witness()["peer"]["trace"] == "otlp:" + second["traceId"]
            assert sum(c["occurrences"] for c in rows()) == 2
            assert not get(base, "findings", kind="shared_identifier", user="someone-else")["rows"]
            if browser:
                import subprocess

                subprocess.run(
                    ["node", Path(__file__).with_name("shared-identifiers-browser.mjs"), base],
                    check=True,
                )
        with sqlite3.connect(database) as db:
            db.execute("DELETE FROM identifier_inputs")
            db.execute("DELETE FROM observations WHERE kind='shared_identifier'")
            db.execute("PRAGMA user_version=39")
        with server(binary, database, *flags) as base:
            assert sum(c["occurrences"] for c in rows()) == 2
            assert witness()["peer"]["trace"] == "otlp:" + second["traceId"]
        with sqlite3.connect(database) as db:
            db.execute("UPDATE traces SET seen=0 WHERE id=?", ("otlp:" + second["traceId"],))
        with server(binary, database, *flags) as base:
            assert witness()["peer"]["trace"] == "otlp:" + first["traceId"]
        with server(binary, Path(directory) / "private.sqlite") as base:
            send_traces(base, export(first, third))
            assert not rows()
    print(
        "Verified two-call UTF-8 evidence, scope, late arrival, retry, expiry, v39 upgrade and redaction."
    )


if __name__ == "__main__":
    check(Path(sys.argv[1]).resolve(), "--browser" in sys.argv)
