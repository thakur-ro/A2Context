"""Return one Qwen2 answer or tool proposal for a shadow trial."""

import argparse
import json
from pathlib import Path
import sys

from probe_transformers_answer_timeline import load_runtime


def propose(runtime, request, model_id, seed=None):
    if request.keys() - {"messages", "tools", "model", "temperature", "max_tokens"}:
        raise ValueError("This runner supports Qwen2 text messages and tool definitions.")
    if request.get("model", model_id) != model_id:
        raise ValueError("The request names a different model from the runner.")
    tools = request.get("tools")
    prompt, tokens, _, _ = runtime.prompt_input(request["messages"], tools)
    answer, _ = runtime.generate(prompt, tokens, request, seed=seed)
    if tools:
        tool, _ = runtime.parse_proposal(answer, tools)
        if tool:
            return {"kind": "tool_call", "label": tool["name"], "outcome": tool["arguments"]}
    return {"kind": "model_output", "label": "Model answer", "outcome": answer}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--model-id", required=True)
    parser.add_argument("--seed", type=int, help="Reset multinomial sampling to this seed for the request.")
    args = parser.parse_args()
    request = json.load(sys.stdin)
    runtime = load_runtime(args.model)
    with runtime.torch.inference_mode():
        choice = propose(runtime, request, args.model_id, args.seed)
    print(json.dumps(choice, allow_nan=False))


if __name__ == "__main__":
    main()
