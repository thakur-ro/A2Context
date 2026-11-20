"""Render text-chat requests with a local Transformers tokenizer. No model weights are loaded."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import sys

from jinja2.exceptions import TemplateError
from transformers import AutoTokenizer, __version__


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("snapshot", type=Path)
    parser.add_argument("--renderer-id", required=True)
    args = parser.parse_args()
    batch = json.load(sys.stdin)
    if batch["version"] != 1:
        raise ValueError("Unsupported rendering batch.")
    tokenizer = AutoTokenizer.from_pretrained(args.snapshot, local_files_only=True, trust_remote_code=False)
    template = (args.snapshot / "chat_template.jinja").read_text()
    if tokenizer.chat_template != template or not tokenizer.is_fast:
        raise ValueError("This adapter requires a fast tokenizer with one local chat_template.jinja.")
    metadata = {"transformers": __version__, "template": template, "tokenizer": tokenizer.backend_tokenizer.to_str(),
                "special_tokens": tokenizer.special_tokens_map, "bos_token": tokenizer.bos_token}
    sequences = []
    for request in batch["requests"]:
        if request.keys() - {"messages", "tools", "model", "temperature", "max_tokens"}:
            raise ValueError("Use the workflow's own renderer for additional model input fields.")
        try:
            prompt = tokenizer.apply_chat_template(request["messages"], tools=request.get("tools"),
                                                  tokenize=False, add_generation_prompt=True)
            bos = tokenizer.bos_token
            ids = tokenizer.encode(prompt, add_special_tokens=bos is None or not prompt.startswith(bos))
        except (KeyError, IndexError, TypeError, ValueError, TemplateError):
            ids = None
        sequences.append(ids)
    config_hash = sha256(json.dumps(metadata, ensure_ascii=False, sort_keys=True).encode()).hexdigest()
    json.dump({"renderer": args.renderer_id, "renderer_sha256": config_hash, "tokens": sequences}, sys.stdout)


if __name__ == "__main__":
    main()
