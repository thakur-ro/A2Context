"""One-off validation runner for the local return-window experiment."""
import json
import os
from pathlib import Path
import re
import sys

os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"

from mlx_lm import generate, load
from mlx_lm.sample_utils import make_sampler

request = json.load(sys.stdin)
model_dir = Path(sys.argv[1])
if not model_dir.is_dir():
    raise SystemExit("A local model directory is required")
model, tokenizer = load(str(model_dir))
prompt = tokenizer.apply_chat_template(
    request["messages"], tokenize=False, add_generation_prompt=True
)
answer = generate(
    model, tokenizer, prompt=prompt, max_tokens=request["max_tokens"],
    sampler=make_sampler(temp=request["temperature"]), verbose=False,
)
match = re.fullmatch(r"\s*(\d+)(?:\s+days?)?\.?\s*", answer, flags=re.IGNORECASE)
if not match:
    raise SystemExit("Model did not return a plain number: " + repr(answer))
json.dump({"kind": "model_output", "label": "Return window", "outcome": int(match[1])}, sys.stdout)
