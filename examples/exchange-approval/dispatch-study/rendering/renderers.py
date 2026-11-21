"""Build equivalent requests for the native Nous and Qwen tokenizer renderers."""
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "prospective"))
from run import LocalGeneration, demo, digest
from scenarios import workflow

SINGLE = '{"name": <function-name>, "arguments": <args-json-object>}'
DOUBLE = '{' + SINGLE + '}'


def tokenizer_request(workflow_request, nous_history):
    original = workflow_request["messages"]
    system = nous_history[0]["content"]
    if ([msg["role"] for msg in original] != [msg["role"] for msg in nous_history]
            or nous_history[1:] != original[1:]
            or not system.startswith(original[0]["content"] + "\n\n# Tools\n")):
        raise ValueError("The workflow transcript or position of native tool instructions changed.")
    start = "\n<tools>\n"
    end = "\n</tools>\n"
    if system.count(start) != 1 or system.count(end) != 1:
        raise ValueError("Cannot isolate the Nous tool schema block.")
    tools = [json.loads(line) for line in system.split(start)[1].split(end)[0].splitlines()]
    expected = json.loads(json.dumps(workflow_request["tools"]))
    for tool in expected:
        if tool["function"]["parameters"].pop("additionalProperties") is not False:
            raise ValueError("Unexpected workflow schema.")
    if tools != expected:
        raise ValueError("Effective function names, order, descriptions, fields or required keys differ.")
    return original, tools


class TokenizerTools(LocalGeneration):
    def model_inputs(self, history):
        request, _ = workflow(self.case, self.arm, demo.TOOLS, demo.MODEL)
        return tokenizer_request(request, history)


def manifest(source, tokenizer):
    rows = []
    for case in source["cases"]:
        for name, arm in case["arms"].items():
            history = json.loads(arm["pending_request"]["attributes"]["a2context.request.body"])["messages"]
            messages, tools = tokenizer_request(arm["workflow_request"], history)
            baseline = tokenizer.apply_chat_template(history, tools=None, tokenize=False, add_generation_prompt=True)
            variant = tokenizer.apply_chat_template(messages, tools=tools, tokenize=False, add_generation_prompt=True)
            if (baseline != arm["generation"]["rendered_prompt"] or baseline.count(SINGLE) != 1
                    or variant != baseline.replace(SINGLE, DOUBLE)):
                raise ValueError("A difference beyond the two registered brace bytes was observed.")
            row = {"case": case["id"], "consent_arm": name, "effective_tools_sha256": digest(tools)}
            for renderer, prompt in (("nous", baseline), ("tokenizer", variant)):
                tokens = tokenizer.encode(prompt, add_special_tokens=False)
                row[renderer] = {"prompt_sha256": digest(prompt), "prompt_tokens_sha256": digest(tokens),
                                 "input_tokens": len(tokens)}
            rows.append(row)
    return rows
