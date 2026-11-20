"""Generate or verify answers for the frozen evidence edits and matched controls."""
import argparse
import json
from pathlib import Path
from time import perf_counter

from selection import HERE, digest, file_hash, inventories, manifest, records
from run import PINNED, load_runtime, request
from check_passage_edits import contains_reference

MANIFEST_SHA = "4f3717abe50381cf40f967f205e2b59382e780b53dca1606ff018d5eb2a3b8d0"
ARMS = ("original", "target", "control")
LIMIT = 48
OMITTED = "[passage omitted]"


def messages_for(record, index, slots):
    _, texts = next(inventories(record, index))
    passages = [OMITTED if i in slots else text for i, text in enumerate(texts, 1)]
    return request(record["question"], passages, None)


def decode(runtime, messages, aliases):
    start = perf_counter()
    prompt, ids, _, _ = runtime.prompt_input(messages, None)
    inputs = runtime.torch.tensor([ids])
    output = runtime.model.generate(
        input_ids=inputs, attention_mask=runtime.torch.ones_like(inputs),
        generation_config=runtime.generation_config, max_new_tokens=LIMIT, use_cache=True,
    )[0, len(ids):].tolist()
    eos = output[-1] in runtime.generation_config.eos_token_id
    answer = runtime.tokenizer.decode(output[:-1] if eos else output, skip_special_tokens=False)
    return {"text": answer, "stop_token_id": output[-1] if eos else None, "prompt_sha256": digest(prompt),
            "prompt_tokens": len(ids), "generated_tokens": len(output), "seconds": perf_counter() - start,
            "alias_match": contains_reference(answer, aliases) if eos else None}


def summary(cases):
    changes = {"target_only": 0, "control_only": 0, "both": 0, "neither": 0}
    category = {(True, False): "target_only", (False, True): "control_only",
                (True, True): "both", (False, False): "neither"}
    same_edit = 0
    for row in cases:
        if any(row[arm]["stop_token_id"] is None for arm in ARMS):
            continue
        original, target, control = [row[arm]["text"] for arm in ARMS]
        changes[category[target != original, control != original]] += 1
        same_edit += target == control
    arms = {}
    for name in ARMS:
        attempts = [r[name] for r in cases]
        complete = [a for a in attempts if a["stop_token_id"] is not None]
        arms[name] = {"calls": len(attempts), "completed": len(complete), "truncated": len(attempts) - len(complete),
                      "abstained": sum(a["text"].strip() == "UNKNOWN" for a in complete),
                      "alias_matched": sum(a["alias_match"] for a in complete),
                      "prompt_tokens": sum(a["prompt_tokens"] for a in attempts),
                      "generated_tokens": sum(a["generated_tokens"] for a in attempts),
                      "seconds": sum(a["seconds"] for a in attempts)}
    return {"complete_triplets": sum(changes.values()), "change_from_original": changes,
            "target_control_same_answer": same_edit, "arms": arms}


def measure(rows, plans, snapshot):
    for name, expected in PINNED.items():
        if file_hash(snapshot / name) != expected:
            raise ValueError(f"Unexpected model file: {name}")
    frozen = json.loads((HERE.parent / "scores.json").read_text())
    runtime = load_runtime(snapshot)
    if runtime.identity != frozen["runtime"]:
        raise ValueError("Runtime differs from the previous experiment.")
    cases = []
    with runtime.torch.inference_mode():
        for index, plan in enumerate(plans):
            if "excluded" in plan:
                continue
            row = plan["row"]
            case = {"row": row}
            for arm in ARMS:
                messages = messages_for(rows[row], index, plan.get(arm, []))
                case[arm] = decode(runtime, messages, rows[row]["answers"])
            cases.append(case)
    result = {key: frozen[key] for key in ("model", "revision", "runtime")}
    result.update(manifest_sha256=MANIFEST_SHA, cases=cases, summary=summary(cases))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dataset", type=Path)
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--check", type=Path)
    args = parser.parse_args()
    path = HERE / "manifest.json"
    if file_hash(path) != MANIFEST_SHA:
        raise ValueError("The frozen selection changed.")
    selection = json.loads(path.read_text())
    if selection["protocol_sha256"] != file_hash(HERE / "protocol.md"):
        raise ValueError("The frozen protocol changed.")
    rows = records(args.dataset) if args.dataset else None
    if rows is not None and manifest(rows) != selection:
        raise ValueError("Source data do not reproduce the frozen edits.")
    if args.check:
        if args.snapshot or args.output:
            parser.error("--check accepts a results file and optional --dataset")
        data = json.loads(args.check.read_text())
        cases = data["cases"]
        expected = [plan["row"] for plan in selection["cases"] if "excluded" not in plan]
        if (data["manifest_sha256"] != MANIFEST_SHA or [c["row"] for c in cases] != expected
                or summary(cases) != data["summary"]):
            raise ValueError("Cases or counts disagree with the frozen plan.")
        for case in cases:
            for arm in ARMS:
                result = case[arm]
                incomplete = result["stop_token_id"] is None
                if not 1 <= result["generated_tokens"] <= LIMIT:
                    raise ValueError("Generated token count falls outside the protocol.")
                if incomplete and (result["generated_tokens"] != LIMIT or result["alias_match"] is not None):
                    raise ValueError("Truncated outputs must retain their 48-token prefix and missing score.")
                if rows is not None and not incomplete:
                    match = contains_reference(result["text"], rows[case["row"]]["answers"])
                    if match != result["alias_match"]:
                        raise ValueError("Alias count differs from the upstream answer list.")
        print(json.dumps(data["summary"], indent=2))
        return
    if rows is None or not args.snapshot or not args.output or args.output.exists():
        parser.error("Provide --dataset, --snapshot and an unused --output path.")
    data = measure(rows, selection["cases"], args.snapshot)
    with args.output.open("x") as out:
        json.dump(data, out, indent=2, allow_nan=False, ensure_ascii=False)
        out.write("\n")


if __name__ == "__main__":
    main()
