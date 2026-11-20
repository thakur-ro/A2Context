"""Check the archived second-runner controls, repeat and source-ranking eligibility."""
from hashlib import sha256
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main():
    rows = json.loads((HERE / "results.json").read_text())
    repeat = json.loads((HERE / "verification.json").read_text())
    selection = json.loads((HERE.parent / "selection.json").read_text())
    for key, path in [("files","files.json"),("protocol","protocol.md"),("selection","../selection.json")]:
        assert rows["design"][key+"_sha256"] == sha256((HERE / path).read_bytes()).hexdigest()
    assert repeat["results_sha256"] == sha256((HERE / "results.json").read_bytes()).hexdigest()
    assert repeat["matched"] == [c["id"] for c in selection["cases"]]
    for ref, row, control in zip(selection["cases"],rows["cases"],rows["controls"],strict=True):
        assert row["id"] == ref["id"] == control["id"]
        assert control["positive"]["usable"] and not control["no_call"]["required_actions"]
        assert not control["no_call"]["required_responses"] and not control["no_call"]["calls"]
        assert len(control["positive"]["calls"]) == len(ref["reference_calls"])
        assert control["positive"]["state_sha256"] == ref["reference_state_sha256"]
        assert row["state_match"] == (row["state_sha256"] == ref["reference_state_sha256"])
        assert row["usable"] == (row["state_match"] and row["required_actions"] and row["required_responses"] and row["stop"] == "normal")
    gs = [g for row in rows["cases"] for g in row["generations"]]
    for k,v in [("calls",len(gs)),("input_tokens",sum(g["input_tokens"] for g in gs)),("generated_tokens",sum(g["generated_tokens"] for g in gs))]:
        assert repeat["cost"][k] == rows["summary"]["cost"][k] == v
    eligible = [r["id"] for r in rows["cases"] if r["usable"]]
    assert not eligible, "A qualifying case requires the registered source-ranking comparison."
    print(f"Verified {len(rows['cases'])} LFM travel baselines, their controls and independent repeat. Zero cases qualified for source edits.")


if __name__ == "__main__":
    main()
