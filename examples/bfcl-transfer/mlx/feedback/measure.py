"""Measure the second inference and resulting simulator states after changing the tool feedback."""
import argparse
from hashlib import sha256
import json
import math
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from runner import load_model, trial
from inputs import digest, new_sim, select
from run import confirm, save, summarize
from replay import logger

FEEDBACK = {"original":"The travel simulator returned ValueError.", "native":"No available route for the given airports.",
            "generic":"The travel simulator could not complete this request."}
NAMES = ("authenticate_travel","get_flight_cost")


def readout(model, tokenizer, prompt, ids):
    import mlx.core as mx

    def encode(s):
        return tokenizer.encode(s, add_special_tokens=tokenizer.bos_token is None or not prompt.startswith(tokenizer.bos_token))
    tag = tokenizer.tool_call_start
    prefix = encode(prompt + tag)
    if prefix[:-1] != ids:
        raise ValueError("Expected a single model-native opening tag after the unchanged prompt tokens.")
    scores, token_counts = {}, {}
    for name in NAMES:
        tokens = encode(prompt + tag + "[" + name + "(")
        if tokens[:len(prefix)] != prefix or len(tokens) == len(prefix):
            raise ValueError("Tool continuation crossed the supplied prefix.")
        # Position j-1 scores token j. Include the unforced tag at position len(ids).
        logits = model(mx.array([tokens[:-1]]))[0,len(ids)-1:].astype(mx.float32)
        lp = logits - mx.logsumexp(logits, axis=-1, keepdims=True)
        chosen = mx.take_along_axis(lp, mx.array(tokens[len(ids):])[:,None], axis=1)[:,0]
        scores[name] = float(mx.sum(chosen[1:]).item())
        token_counts[name] = len(tokens)-len(prefix)
        if name == NAMES[0]:
            tag_probability = math.exp(float(chosen[0].item()))
    return {"prompt_sha256":sha256(prompt.encode()).hexdigest(), "input_tokens":len(ids),
            "opening_probability":tag_probability, "name_log_probabilities":scores, "continuation_tokens":token_counts,
            "authentication_minus_fare":scores[NAMES[0]] - scores[NAMES[1]]}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for flag in ("snapshot","upstream","output"):
        p.add_argument("--"+flag, type=Path, required=True)
    p.add_argument("--repeat", type=Path)
    args = p.parse_args()
    if args.output.exists():
        p.error("Use a fresh output path.")
    _, packets, schemas, cls = select(args.upstream)
    pkt = next(x for x in packets if x["id"] == "multi_turn_base_150")
    baseline_file = HERE.parent / "results.json"
    base = next(x for x in json.loads(baseline_file.read_text())["cases"] if x["id"] == pkt["id"])
    model, tokenizer = load_model(args.snapshot)
    report = {"id":pkt["id"],"baseline_sha256":sha256(baseline_file.read_bytes()).hexdigest(),
              "protocol_sha256":sha256((HERE/"protocol.md").read_bytes()).hexdigest(),"arms":{}}
    old = json.loads(args.repeat.read_text()) if args.repeat else None
    initial_ids = None
    for arm, text in FEEDBACK.items():
        audit = []
        scores = []
        def before_second(prompt, ids):
            scores.append(readout(model, tokenizer, prompt, ids))
        row = trial(model, tokenizer, pkt, schemas, cls, audit=audit, first_feedback=text, before_second=before_second)
        if arm == "original":
            confirm(row, base, tokenizer)
        first, = audit[0]["callbacks"]
        if (first["name"] != NAMES[1] or first["returned"] != FEEDBACK["original"] or first["before"] != first["after"]
                or row["calls"][0] != base["calls"][0]):
            raise ValueError("First call or error differed from the calibrated checkpoint.")
        try:
            new_sim(cls,pkt["state"]).get_flight_cost(**first["arguments"])
        except ValueError as exc:
            if str(exc) != FEEDBACK["native"]:
                raise ValueError("Pinned simulator error changed.") from exc
        else:
            raise ValueError("The original call must fail.")
        if row["generations"][0]["raw_sha256"] != base["generations"][0]["raw_sha256"] or (initial_ids is not None and audit[0]["tokens"] != initial_ids):
            raise ValueError("The first unchanged prompt or completion differed between arms.")
        initial_ids = audit[0]["tokens"]
        score, = scores
        if score["prompt_sha256"] != row["generations"][1]["prompt_sha256"]:
            raise ValueError("The score must use the generated arm’s second prompt.")
        changes = [[{k:c[k] for k in ("name","before","after")} for c in step["callbacks"]] for step in audit]
        report["arms"][arm] = {"score":score, "trajectory":row,"callback_states":changes}
        if old:
            previous = old["arms"][arm]
            confirm(row, previous["trajectory"], tokenizer)
            if changes != previous["callback_states"] or score != previous["score"]:
                raise ValueError("The checkpoint scores or state path differed on repeat.")
    if old:
        if any(old[k] != report[k] for k in ("id","baseline_sha256","protocol_sha256")):
            raise ValueError("Wrong baseline or study.")
        save(args.output, {"report_sha256":sha256(args.repeat.read_bytes()).hexdigest(),"matched":list(FEEDBACK),
                           "cost":summarize([x["trajectory"] for x in report["arms"].values()])["cost"]})
    else:
        save(args.output, report)


if __name__ == "__main__":
    logger.setLevel("ERROR")
    main()
