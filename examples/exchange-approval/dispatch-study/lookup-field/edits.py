"""Prepare a one-digit lookup edit and a one-character edit in an older user message."""
import argparse
from copy import deepcopy
from hashlib import sha256
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "prefill-tool"))
from freeze import PREFIX, digest
sys.path.insert(0, str(HERE.parent / "prospective"))
from run import DESIGN, demo, pending, workflow

ARMS = ("original", "lookup", "punctuation")


def alternative(value):
    return value[:-1] + str((int(value[-1]) + 1) % 10)


def edited_history(history, case, arm):
    messages = deepcopy(history)
    value = case["pending_arguments"]["payment_method_id"]
    if arm == "lookup":
        before = "payment_method_id=" + value
        if messages[0]["content"].count(before) != 1:
            raise ValueError("Expected exactly one target field in the system lookup.")
        messages[0]["content"] = messages[0]["content"].replace(before, "payment_method_id=" + alternative(value))
    elif arm == "punctuation":
        msg = messages[1]
        if msg["role"] != "user" or not msg["content"].endswith("."):
            raise ValueError("Expected a sentence-final period in the first customer request.")
        msg["content"] = msg["content"][:-1] + "!"
    elif arm != "original":
        raise ValueError("Unknown source edit.")
    return messages


def envelope(case, messages, arm):
    _, cache = workflow(case, "repair", demo.TOOLS, demo.MODEL)
    req = pending(case, "repair", messages, cache)
    value = case["pending_arguments"]["payment_method_id"]
    value = alternative(value) if arm == "lookup" else value
    content = messages[0]["content"]
    if content.count(value) != 1:
        raise ValueError("Expected unique payment field in the model-facing system message.")
    start = len(content[:content.index(value)].encode())
    req["attributes"]["a2context.context.semantics"]["request_sources"].append({
        "id": "payment-lookup", "source_id": "payment-lookup", "source_version": "captured",
        "role": "system", "request_fragment": demo.fragment("/messages/0/content", value, start)
    })
    req["attributes"]["a2context.required_sources"] = {
        "name": "Authorized payment method", "any_of": [["payment-lookup"]],
        "expected_text_sha256": {"payment-lookup": digest(case["pending_arguments"]["payment_method_id"])}
    }
    req["attributes"]["exchange.assistant_prefix"] = PREFIX
    req["attributes"]["exchange.remaining_output_tokens"] = 126
    return req


def prepare(tokenizer):
    source = (HERE.parent / "prospective/results.json").read_bytes()
    prefix_source = (HERE.parent / "prefill-tool/results.json").read_bytes()
    cases = json.loads(source)["cases"]
    prefix_ids = tokenizer.encode(PREFIX, add_special_tokens=False)
    rows = []
    for case, saved in zip(DESIGN, cases, strict=True):
        parent = saved["arms"]["repair"]
        history = json.loads(parent["pending_request"]["attributes"]["a2context.request.body"])["messages"]
        value = case["pending_arguments"]["payment_method_id"]
        old_prompt = parent["generation"]["rendered_prompt"] + PREFIX
        if old_prompt.count(value) != 1 or alternative(value) in old_prompt or not old_prompt.isascii():
            raise ValueError("Target is ambiguous or its substitute already appears.")
        old_tokens = tokenizer(old_prompt, add_special_tokens=False, return_offsets_mapping=True)
        encoded = old_tokens["input_ids"]
        result = {"case": case["id"], "authentic": value, "substitute": alternative(value),
                  "authentic_sha256": digest(value), "arms": {}}
        for arm in ARMS:
            msgs = edited_history(history, case, arm)
            req = envelope(case, msgs, arm)
            key = "a2context.context.semantics"
            fragment = req["attributes"][key]["request_sources"][-1]["request_fragment"]
            plain = tokenizer.apply_chat_template(msgs, tools=None, tokenize=False, add_generation_prompt=True)
            prompt = plain + PREFIX
            tokens = tokenizer(prompt, add_special_tokens=False, return_offsets_mapping=True)
            if len(prompt) != len(old_prompt) or len(encoded) != len(tokens.input_ids):
                raise ValueError("Registered edit must preserve prompt bytes and token count.")
            if tokenizer.encode(plain, add_special_tokens=False) + prefix_ids != tokens.input_ids:
                raise ValueError("Assistant prefix did not append whole tokens.")
            bytes_changed = [i for i, (a, b) in enumerate(zip(old_prompt, prompt)) if a != b]
            ids_changed = [i for i, (a, b) in enumerate(zip(encoded, tokens.input_ids)) if a != b]
            if len(bytes_changed) != int(arm != "original") or len(ids_changed) != len(bytes_changed):
                raise ValueError("Expected one changed byte and token, or an unmodified baseline.")
            full_value = alternative(value) if arm == "lookup" else value
            first = prompt.index(full_value)
            field_range = [idx for idx, (a, b) in enumerate(tokens.offset_mapping)
                           if a < first + len(full_value) and b > first]
            entry = {"prompt_sha256": digest(prompt), "tokens_sha256": digest(tokens.input_ids),
                     "input_tokens": len(tokens.input_ids), "field_fragment": fragment,
                     "field_prompt_bytes": [first, first + len(full_value)],
                     "field_prompt_tokens": [field_range[0], field_range[-1] + 1]}
            if arm != "original":
                pos, index = bytes_changed[0], ids_changed[0]
                if not tokens.offset_mapping[index][0] <= pos < tokens.offset_mapping[index][1]:
                    raise ValueError("Changed byte lies outside the sole changed token.")
                message_index = 0 if arm == "lookup" else 1
                content = history[message_index]["content"]
                local = content.index(value) + len(value) - 1 if arm == "lookup" else len(content) - 1
                if old_prompt.index(content) + local != pos:
                    raise ValueError("Model input edit does not align with its source span.")
                entry["change"] = {"message_pointer": f"/messages/{message_index}/content", "message_byte": local,
                                   "prompt_byte": pos, "before_byte": old_prompt[pos], "after_byte": prompt[pos],
                                   "token_index": index, "before_id": encoded[index], "after_id": tokens.input_ids[index]}
            result["arms"][arm] = entry
        rows.append(result)
    return {"protocol_sha256": sha256((HERE / "protocol.md").read_bytes()).hexdigest(),
            "source_sha256": sha256(source).hexdigest(), "prefix_results_sha256": sha256(prefix_source).hexdigest(),
            "prefix": PREFIX, "prefix_ids": prefix_ids, "assistant_budget": 128, "cases": rows}


if __name__ == "__main__":
    from transformers import AutoTokenizer
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("snapshot", type=Path)
    args = p.parse_args()
    print(json.dumps(prepare(AutoTokenizer.from_pretrained(args.snapshot, local_files_only=True, trust_remote_code=False)), indent=2))
