"""Swap one aligned Qwen2 decoder output at a fixed assistant prefix."""

from hashlib import sha256
import json
import re

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
    pointer = re.fullmatch(r"/messages/([0-9]+)/content", change["pointer"])
    if not pointer or change["operation"] != "replace_text_fragment":
        raise ValueError("Expected a text replacement within a captured message.")
    message = body["messages"][int(pointer[1])]
    content = message["content"].encode()
    start, end = (change["text_range"][key] for key in ("start_byte", "end_byte"))
    message["content"] = (content[:start] + change["replacement"].encode() + content[end:]).decode()
    return json.dumps(body, ensure_ascii=False, separators=(",", ":"))


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



def measure(runtime, raw, candidate):
    scores = candidate["scores"]
    identity = ("model_id", "model_revision", "runtime", "tokenizer_sha256")
    ids = [tokens[0] for tokens in scores["candidate_token_ids"]]
    if runtime.identity != scores["runtime"] or ids[0] == ids[1]:
        raise ValueError("Expected the scored runtime and two distinct next tokens.")
    original, baseline = condition(runtime, raw, candidate, scores["conditions"][0], candidate["captured_request_sha256"])
    receiver_states = capture(runtime, original, baseline, ids)
    controls = sweep(runtime, original, baseline, ids, receiver_states)
    if any(abs(x - baseline["margin"]) > TOLERANCE for row in controls.values() for x in row):
        raise ValueError("A self-transplant changed the measured margin.")
    effects = []
    for effect, score in zip(candidate["effects"], scores["conditions"][1:len(candidate["effects"]) + 1], strict=True):
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
