"""Run single-position Qwen residual transplants on the frozen booking questions."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from check_question_holdout import compact, verify_model
from probe_transformers_answer_timeline import load_runtime

PROTOCOL = HERE / "protocol.json"
CASES = HERE.parent / "prefix-indirect/cases.json"
SITES = ("last_user_token", "last_prefix_token")
TOLERANCE = 0.0001


def digest(text):
    return sha256(text.encode()).hexdigest()


def condition(runtime, body, candidate, score, request_hash):
    request = json.loads(body)
    rendered = runtime.render_prompt(request["messages"], request.get("tools"))
    text = rendered + candidate["assistant_prefix"]
    question = request["messages"][-1]
    if question["role"] != "user" or rendered.count(question["content"]) != 1:
        raise ValueError("Expected one identifiable final user message in the rendered prompt.")
    end = rendered.index(question["content"]) + len(question["content"])
    encoded = runtime.tokenizer(text, add_special_tokens=False, return_offsets_mapping=True)
    user_index, = [i for i, (start, stop) in enumerate(encoded.offset_mapping) if start < end and stop == end]
    tokens = encoded.input_ids
    sites = [user_index, len(tokens) - 1]
    if digest(body) != request_hash or digest(text) != score["rendered_prefix_sha256"]:
        raise ValueError("Input bytes or rendered prefix differ from the scored condition.")
    prefix_ids = candidate["scores"]["assistant_prefix_token_ids"]
    if not prefix_ids or tokens[-len(prefix_ids):] != prefix_ids:
        raise ValueError("The held prefix differs from the original scoring run.")
    for choice, ids in zip(candidate["candidates"], candidate["scores"]["candidate_token_ids"], strict=True):
        if runtime.tokenizer.encode(text + choice["continuation"], add_special_tokens=False) != tokens + ids:
            raise ValueError("The candidate tokenization differs at this prefix.")
    return tokens, {
        "request_sha256": request_hash, "rendered_prefix_sha256": digest(text), "token_count": len(tokens),
        "positions": {site: {"index": pos, "token_id": tokens[pos]} for site, pos in zip(SITES, sites)},
    }


def edited_body(raw, effect):
    body = json.loads(raw)
    change = effect["intervention"]
    if change["pointer"] != "/messages/0/content" or change["operation"] != "replace_text_fragment":
        raise ValueError("Expected the frozen source replacement in the system message.")
    content = body["messages"][0]["content"].encode()
    start, end = (change["text_range"][key] for key in ("start_byte", "end_byte"))
    body["messages"][0]["content"] = (content[:start] + change["replacement"].encode() + content[end:]).decode()
    return compact(body)


def capture(runtime, tokens, origin, ids):
    positions = [origin["positions"][site]["index"] for site in SITES]
    states = {}

    def save(block):
        def observe(hidden):
            states[block] = hidden[0, positions, :].clone()
            return hidden
        return observe

    origin["margin"], = runtime.scores(tokens, [(len(tokens) - 1, *ids)],
                                       {i: save(i) for i in range(runtime.layer_count)})
    return states


def sweep(runtime, receiver, original, ids, donor_states):
    margins = {}
    for site_index, site in enumerate(SITES):
        position = original["positions"][site]["index"]
        values = []
        for block in range(runtime.layer_count):
            replacement = donor_states[block][site_index]

            def transplant(hidden):
                result = hidden.clone()
                result[0, position, :] = replacement
                return result

            value, = runtime.scores(receiver, [(len(receiver) - 1, *ids)], {block: transplant})
            values.append(value)
        margins[site] = values
    return margins


def measure(runtime, record, pinned):
    envelope = record["envelope"]
    candidate = envelope["candidate"]
    scores = candidate["scores"]
    identity = ("model_id", "model_revision", "runtime", "tokenizer_sha256")
    if any(scores[k] != pinned[p] for k, p in zip(identity, ("checkpoint", "revision", "runtime", "tokenizer_sha256"))) or runtime.identity != scores["runtime"]:
        raise ValueError("The scorer identity differs from the pinned experiment.")
    ids = [tokens[0] for tokens in scores["candidate_token_ids"]]
    if [runtime.tokenizer.decode([i]) for i in ids] != ["3", "6"] or runtime.layer_count != 24:
        raise ValueError("The readout or decoder blocks differ from the protocol.")
    raw = envelope["request"]["attributes"]["nexusweave.request.body"]
    original, baseline = condition(runtime, raw, candidate, scores["conditions"][0], candidate["captured_request_sha256"])
    receiver_states = capture(runtime, original, baseline, ids)
    controls = sweep(runtime, original, baseline, ids, receiver_states)
    if any(abs(x - baseline["margin"]) > TOLERANCE for row in controls.values() for x in row):
        raise ValueError("A self-transplant changed the measured margin.")
    effects = []
    for effect, score in zip(candidate["effects"], scores["conditions"][1:], strict=True):
        tokens, donor = condition(runtime, edited_body(raw, effect), candidate, score, effect["request_sha256"])
        if score["id"] != effect["condition_id"] or any(
            donor["positions"][site]["token_id"] != baseline["positions"][site]["token_id"] for site in SITES
        ):
            raise ValueError("The source or aligned token differs across arms.")
        states = capture(runtime, tokens, donor, ids)
        swaps = sweep(runtime, original, baseline, ids, states)
        if abs(swaps[SITES[1]][-1] - donor["margin"]) > TOLERANCE or abs(swaps[SITES[0]][-1] - baseline["margin"]) > TOLERANCE:
            raise ValueError("A final-block control failed.")
        effects.append({"condition_id": score["id"], "donor": donor, "swaps": swaps})
    return {"version": 1, **{k: scores[k] for k in identity}, "blocks": runtime.layer_count,
            "choice_token_ids": ids, "baseline": baseline, "self_swaps": controls, "effects": effects}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    protocol = json.loads(PROTOCOL.read_text())
    pinned = json.loads((HERE / protocol["model_protocol"]).read_text())
    if sha256(CASES.read_bytes()).hexdigest() != protocol["source_cases_sha256"]:
        raise ValueError("The original case file differs from the frozen protocol.")
    verify_model(args.model)
    if sha256((args.model / "tokenizer.json").read_bytes()).hexdigest() != pinned["tokenizer_sha256"]:
        raise ValueError("The tokenizer differs from the pinned tokenizer.")
    runtime = load_runtime(args.model)
    lookup = {r["id"]: r for r in json.loads(CASES.read_text())}
    reports = {}
    with runtime.torch.inference_mode():
        for key in protocol["cases"]:
            reports[key] = measure(runtime, lookup[key], pinned)
            print("Measured", key, flush=True)
    with args.output.open("x") as out:
        json.dump({"protocol_sha256": sha256(PROTOCOL.read_bytes()).hexdigest(), "reports": reports},
                  out, indent=2, allow_nan=False)
        out.write("\n")


if __name__ == "__main__":
    main()
