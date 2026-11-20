"""Verify the crossed design and its controls using only the saved receipts."""
from hashlib import sha256
from itertools import product
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def verify():
    path = HERE / "results.json"
    saved = json.loads(path.read_text())
    parent = HERE.parent / "results.json"
    assert saved["parent_sha256"] == sha256(parent.read_bytes()).hexdigest()
    assert saved["protocol_sha256"] == sha256((HERE / "protocol.md").read_bytes()).hexdigest()
    assert json.loads((HERE / "verification.json").read_text()) == {
        "report_sha256": sha256(path.read_bytes()).hexdigest(), "matched_cells": 16}
    previous, = [case for case in json.loads(parent.read_text())["cases"] if case["id"] == saved["task"]]
    assert previous["id"] == "multi_turn_base_132"
    sources = saved["source_messages_sha256"]
    assert [i for i, (a, b) in enumerate(zip(sources["full"], sources["scoped"])) if a != b] == [2, 4]
    catalogs = ("full", "scoped", "dots", "letters")
    histories = list(product(("full", "scoped"), repeat=2))
    cells = {(c["catalog"], c["first"], c["second"]): c for c in saved["cells"]}
    assert len(cells) == len(saved["cells"]) == 16
    assert len({c["prompt_sha256"] for c in cells.values()}) == 16
    for first, second in histories:
        base = cells["full", first, second]
        for catalog in catalogs:
            cell = cells[catalog, first, second]
            assert cell["input_tokens"] == base["input_tokens"] - (2465 if catalog == "scoped" else 0)
            assert 0 <= cell["opener_probability"] <= 1
            assert cell["finish_reason"] == "stop"
            if cell["outcome"] == "get_watchlist":
                assert cell["proposals"] == [{"name": "get_watchlist", "arguments_sha256": sha256(b"{}").hexdigest()}]
            else:
                assert cell["outcome"] == "text" and not cell["proposals"]
            if first == second == ("full" if catalog == "full" else "scoped"):
                natural = previous["arms"][catalog]["generations"][2]
                assert all(cell[k] == natural[k] for k in ("prompt_sha256", "raw_sha256", "history_sha256", "generated_tokens"))
    for catalog in catalogs:
        print(catalog, [(cells[(catalog, *history)]["outcome"], round(cells[(catalog, *history)]["opener_probability"], 4))
                        for history in histories])


if __name__ == "__main__":
    verify()
