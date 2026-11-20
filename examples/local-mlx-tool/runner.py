"""Propose a tool call or answer using a local MLX model."""
import json
import os
from pathlib import Path
import sys

os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"

from mlx_lm import load, stream_generate
from mlx_lm.sample_utils import make_sampler

request = json.load(sys.stdin)
model_dir = Path(sys.argv[1])
if not model_dir.is_dir():
    raise SystemExit("A local model directory is required")
model, tokenizer = load(str(model_dir))
tools = request["tools"]
prompt = tokenizer.apply_chat_template(
    request["messages"], tools=tools, tokenize=False, add_generation_prompt=True,
)
tokens = list(stream_generate(
    model, tokenizer, prompt=prompt, max_tokens=request["max_tokens"],
    sampler=make_sampler(temp=request["temperature"]),
))
if tokens[-1].finish_reason != "stop":
    raise SystemExit("Generation ended before a complete proposal")
answer = "".join(token.text for token in tokens).strip()
if tokenizer.tool_call_start and tokenizer.tool_call_start in answer:
    call = answer.split(tokenizer.tool_call_start, 1)[1]
    payload, end, tail = call.partition(tokenizer.tool_call_end)
    if not end:
        raise SystemExit("Incomplete tool call")
    proposal = tokenizer.tool_parser(payload, tools)
    if isinstance(proposal, list) and len(proposal) == 1:
        proposal = proposal[0]
    if not isinstance(proposal, dict) or tokenizer.tool_call_start in tail:
        raise SystemExit("Expected a single proposed tool call")
    decision = {"kind": "tool_call", "label": proposal["name"], "outcome": proposal["arguments"]}
else:
    if not answer:
        raise SystemExit("No proposed decision")
    decision = {"kind": "model_output", "label": "Model answer", "outcome": answer}
json.dump(decision, sys.stdout)
