"""Verify the saved directory experiment and print paired outcomes without a model or SDK."""
from hashlib import sha256
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def read(name):
    return json.loads((HERE / name).read_text())


def sha(name):
    return sha256((HERE / name).read_bytes()).hexdigest()


def main():
    result, repeat = read("results.json"), read("verification.json")
    baseline = read("../results.json")["cases"]
    assert result["baseline_sha256"] == sha("../results.json")
    assert result["protocol_sha256"] == sha("protocol.md")
    assert repeat["results_sha256"] == sha("results.json")
    case_ids = [base["id"] for base in baseline]
    assert repeat["matched_cases"] == case_ids == [case["id"] for case in result["cases"]]
    fields = [field for case in read("baseline-fields.json") for call in case["calls"]
              for ref in call["matching_name_comparisons"] for field in ref.get("airport_fields", {}).values()]
    assert len(fields) == 8 and not any(f["supported"] for f in fields)
    assert sum(f["resolved_to_reference"] for f in fields) == 7
    arms = {"unchanged":baseline, "withheld":[], "verified":[]}
    generations = []
    for case, base in zip(result["cases"], baseline, strict=True):
        for arm in ("withheld","verified"):
            row = case["arms"][arm]
            if not case["matched_locations"]:
                assert row == {"reused_baseline":True}
                row = base
            else:
                assert row["id"] == case["id"]
                generations.extend(row["generations"])
            assert row["usable"] == (row["stop"] == "normal" and row["state_match"] and row["required_actions"] and row["required_responses"])
            arms[arm].append(row)
    totals = {"calls":len(generations), "input_tokens":sum(g["input_tokens"] for g in generations),
              "generated_tokens":sum(g["generated_tokens"] for g in generations)}
    for key, value in totals.items():
        assert result["cost"][key] == repeat["cost"][key] == value
    for name, rows in arms.items():
        outcomes = {key:sum(row[key] for row in rows) for key in ("usable","required_actions","required_responses","state_match")}
        if name != "unchanged":
            assert outcomes == result["outcomes"][name]
        print(name, outcomes)
    for name in ("unchanged","withheld"):
        pairs = list(zip(arms[name],arms["verified"], strict=True))
        print("verified vs", name, "newly qualified", [a["id"] for a,b in pairs if not a["usable"] and b["usable"]],
              "lost", [a["id"] for a,b in pairs if a["usable"] and not b["usable"]])


if __name__ == "__main__":
    main()
