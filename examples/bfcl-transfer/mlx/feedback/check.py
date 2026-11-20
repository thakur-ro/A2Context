"""Verify the archived intervention against its baseline, callback states and independent repeat."""
from hashlib import sha256
import json
import math
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main():
    report = json.loads((HERE / "results.json").read_text())
    repeat = json.loads((HERE / "verification.json").read_text())
    assert report["protocol_sha256"] == sha256((HERE / "protocol.md").read_bytes()).hexdigest()
    assert report["baseline_sha256"] == sha256((HERE.parent / "results.json").read_bytes()).hexdigest()
    assert repeat["report_sha256"] == sha256((HERE / "results.json").read_bytes()).hexdigest()
    assert repeat["matched"] == list(report["arms"]) == ["original","native","generic"]
    base = next(r for r in json.loads((HERE.parent / "results.json").read_text())["cases"] if r["id"] == report["id"])
    ref = next(r for r in json.loads((HERE.parents[1] / "selection.json").read_text())["cases"] if r["id"] == report["id"])
    assert ref["initial_state_sha256"] == ref["reference_state_sha256"]
    prompts, generations = set(), []
    for arm, record in report["arms"].items():
        score, row = record["score"], record["trajectory"]
        assert row["id"] == base["id"] and row["calls"] == base["calls"]
        assert row["state_sha256"] == base["state_sha256"] != ref["reference_state_sha256"]
        assert row["stop"] == base["stop"] == "completion_bound" and not row["required_actions"] and not row["required_responses"]
        assert [g["raw_sha256"] for g in row["generations"]] == [g["raw_sha256"] for g in base["generations"]]
        assert row["generations"][0]["prompt_sha256"] == base["generations"][0]["prompt_sha256"]
        if arm == "original":
            assert all(g["prompt_sha256"] == b["prompt_sha256"] for g,b in zip(row["generations"],base["generations"],strict=True))
        generations.extend(row["generations"])
        assert row["generations"][1]["prompt_sha256"] == score["prompt_sha256"]
        prompts.add(score["prompt_sha256"])
        assert 0 < score["opening_probability"] <= 1
        p = score["name_log_probabilities"]
        assert math.isclose(p["authenticate_travel"] - p["get_flight_cost"],score["authentication_minus_fare"])
        assert score["continuation_tokens"] == {"authenticate_travel":7, "get_flight_cost":7}
        state = ref["initial_state_sha256"]
        changed = []
        for step in record["callback_states"]:
            for call in step:
                assert call["before"] == state
                state = call["after"]
                if state != call["before"]:
                    changed.append(call["name"])
        assert state == row["state_sha256"] and changed == ["authenticate_travel"]
    assert len(prompts) == 3
    assert repeat["cost"]["calls"] == len(generations)
    for name in ("input_tokens","generated_tokens"):
        assert repeat["cost"][name] == sum(g[name] for g in generations)
    print("Verified three feedback conditions, fixed-prefix scores, identical proposals and one state-changing callback per arm.")


if __name__ == "__main__":
    main()
