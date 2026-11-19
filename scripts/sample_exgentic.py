"""Stream a pinned public retail sample to an ignored local directory."""
import argparse
import hashlib
import json
from pathlib import Path

from huggingface_hub import HfApi, HfFileSystem
from pyarrow.parquet import ParquetFile

REPOSITORY = "Exgentic/agent-llm-traces"
REVISION = "70036b93a04e61b0ea2706a68b962f4f26774587"
BENCHMARK = "tau2_retail"
COUNT = 8


def decode(value):
    return json.loads(value) if isinstance(value, str) else value


def has_linked_result(row):
    requested = set()
    spans = [decode(span) for span in decode(row["spans"])]
    for span in sorted(spans, key=lambda span: (span.get("start_time") or "", span["span_id"])):
        attributes = decode(span.get("attributes") or {})
        inputs = decode(attributes.get("gen_ai.input.messages") or [])
        outputs = decode(attributes.get("gen_ai.output.messages") or [])
        for message in inputs:
            for part in message.get("parts") or []:
                if part.get("type") == "tool_call_response" and part.get("id") in requested:
                    return True
        if inputs:
            requested.update(
                part["id"]
                for message in outputs
                for part in (message.get("parts") or [])
                if part.get("type") == "tool_call" and part.get("id")
            )
    return False


def select_rows(revision, files):
    filesystem = HfFileSystem()
    for name in files:
        remote_path = f"datasets/{REPOSITORY}@{revision}/{name}"
        with filesystem.open(remote_path, "rb", block_size=256 * 1024) as source:
            parquet = ParquetFile(source)
            for group in range(parquet.num_row_groups):
                benchmarks = parquet.read_row_group(group, columns=["benchmark"])["benchmark"]
                if BENCHMARK not in benchmarks.to_pylist():
                    continue
                table = parquet.read_row_group(group, columns=["benchmark", "session_id", "spans"])
                for row in table.to_pylist():
                    if row["benchmark"] == BENCHMARK and has_linked_result(row):
                        yield row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=Path("data"))
    args = parser.parse_args()

    api = HfApi()
    revision = api.dataset_info(REPOSITORY, revision=REVISION).sha
    files = sorted(
        path for path in api.list_repo_files(REPOSITORY, repo_type="dataset", revision=revision)
        if path.endswith(".parquet")
    )
    if not files:
        raise SystemExit("The pinned dataset revision contains no Parquet files.")

    rows = []
    session_ids = set()
    for row in select_rows(revision, files):
        if row["session_id"] not in session_ids:
            rows.append(row)
            session_ids.add(row["session_id"])
        if len(rows) == COUNT:
            break
    if len(rows) != COUNT:
        raise SystemExit(f"Found only {len(rows)} retail sessions with linked tool results.")

    args.output_dir.mkdir(exist_ok=True, parents=True)
    content = "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows).encode("utf-8")
    (args.output_dir / "exgentic-retail.jsonl").write_bytes(content)
    manifest = {
        "dataset": REPOSITORY,
        "revision": revision,
        "benchmark": BENCHMARK,
        "selection": "First eight distinct sessions in sorted shard and row order with a captured request, tool call, and later matching result.",
        "session_ids": [row["session_id"] for row in rows],
        "sha256": hashlib.sha256(content).hexdigest(),
    }
    (args.output_dir / "exgentic-retail.manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    print(f"Selected {len(rows)} sessions at {revision}.")


if __name__ == "__main__":
    main()
