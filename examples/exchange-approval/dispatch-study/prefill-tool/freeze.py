"""Re-render eight saved Nous prompts and register their empty and supplied assistant prefixes."""
import argparse
from hashlib import sha256
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
PREFIX = "<tool_call>\n"
LIMIT = 128


def digest(value):
    if not isinstance(value, str):
        value = json.dumps(value, ensure_ascii=False, separators=(",", ":"))
    return sha256(value.encode()).hexdigest()


def design(tokenizer):
    source = (HERE.parent / "prospective/results.json").read_bytes()
    render_source = (HERE.parent / "rendering/results.json").read_bytes()
    cases = json.loads(source)["cases"]
    native = json.loads(render_source)["pairs"]
    rows = []
    prefix_ids = tokenizer.encode(PREFIX, add_special_tokens=False)
    for case in cases:
        for consent, arm in case["arms"].items():
            pair = native[len(rows)]
            original = arm["generation"]
            if ((pair["case"], pair["consent_arm"]) != (case["id"], consent)
                    or pair["nous"]["generation"]["output_ids"] != original["output_ids"]):
                raise ValueError("Previous native baselines disagree.")
            boundary = json.loads(arm["pending_request"]["attributes"]["a2context.request.body"])
            prompt = tokenizer.apply_chat_template(boundary["messages"], tools=None, tokenize=False,
                                                  add_generation_prompt=True)
            plain = tokenizer.encode(prompt, add_special_tokens=False)
            prefixed = tokenizer.encode(prompt + PREFIX, add_special_tokens=False)
            if (prompt != original["rendered_prompt"] or digest(plain) != original["prompt_tokens_sha256"]
                    or prefixed != plain + prefix_ids or tokenizer.decode(prefix_ids) != PREFIX):
                raise ValueError("The prefix must append complete tokens to precisely the saved prompt.")
            rows.append({"case": case["id"], "consent_arm": consent,
                         "empty": {"prompt_sha256": digest(prompt), "tokens_sha256": digest(plain), "input_tokens": len(plain)},
                         "prefilled": {"prompt_sha256": digest(prompt + PREFIX), "tokens_sha256": digest(prefixed),
                                       "input_tokens": len(prefixed)}})
    if len(rows) != 8 or len(prefix_ids) >= LIMIT:
        raise ValueError("Unexpected number of requests or prefix tokens.")
    return {"protocol_sha256": sha256((HERE / "protocol.md").read_bytes()).hexdigest(),
            "parent_sha256": sha256(source).hexdigest(), "render_results_sha256": sha256(render_source).hexdigest(),
            "prefix": PREFIX, "prefix_ids": prefix_ids, "assistant_budget": LIMIT,
            "suffix_budget": LIMIT - len(prefix_ids), "pairs": rows}


if __name__ == "__main__":
    from transformers import AutoTokenizer
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("snapshot", type=Path)
    args = parser.parse_args()
    tokenizer = AutoTokenizer.from_pretrained(args.snapshot, local_files_only=True, trust_remote_code=False)
    print(json.dumps(design(tokenizer), ensure_ascii=False, indent=2))
