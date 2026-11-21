"""Verify renderer receipts and crossed inventory answers without loading a model."""
from hashlib import sha256
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def verify():
    path = HERE / "results.json"
    data = json.loads(path.read_text())
    assert data["protocol_sha256"] == sha256((HERE / "protocol.md").read_bytes()).hexdigest()
    assert json.loads((HERE / "verification.json").read_text()) == {
        "matched_generations": 8, "report_sha256": sha256(path.read_bytes()).hexdigest()}
    old = json.loads((HERE.parent / "report-result.json").read_text())
    archived = {"original": old["baseline"][0]["choice"]["outcome"], "zero": old["variants"][0]["trials"][0]["choice"]["outcome"]}
    keys = set()
    for package, rendered in data["renderings"].items():
        preview = json.loads((HERE / (package + "-preview.json")).read_text())
        check = preview["attributes"]["a2context.render_check"]
        assert check["request_sha256"] == old["baseline_request_sha256"]
        for name, field in [("original", check["baseline"]), ("omit_tool_calls", check["removals"][0]["tokens"])]:
            assert (field["count"], field["sha256"]) == (rendered[name]["input_tokens"], rendered[name]["ids_sha256"])
        assert (rendered["original"] == rendered["omit_tool_calls"]) == (package == "liquid")
        assert rendered["quantity_17"]["ids_sha256"] != rendered["original"]["ids_sha256"]
    for cell in data["generations"]:
        c, t, variant = cell["checkpoint"], cell["template"], cell["condition"]
        keys.add((c,t,variant))
        assert cell["input_ids_sha256"] == data["renderings"][t][variant]["ids_sha256"]
        text = cell["streamed_text"]
        assert cell["decoded_text"] == text and cell["output_sha256"] == sha256(text.encode()).hexdigest()
        assert text == (archived[variant] if t == "community" else {"original":"2","zero":"0"}[variant])
    assert len(keys) == len(data["generations"]) == 8
    print("Verified both renderer checks, eight crossed completions, and the independent repeat.")


if __name__ == "__main__":
    verify()
