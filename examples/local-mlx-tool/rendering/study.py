"""Cross two verified LFM distributions with their native rendered inventory prompts."""
import argparse
from copy import deepcopy
from hashlib import file_digest, sha256
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[2] / "scripts"))
from mlx_activation import prompt_input


def digest(data):
    return sha256(data).hexdigest()


def tokens_hash(ids):
    return digest(json.dumps(ids, separators=(",", ":")).encode())


def measure(paths):
    from importlib.metadata import version
    from mlx_lm import load, stream_generate
    from mlx_lm.sample_utils import make_sampler

    case = json.loads((HERE.parent / "case-result.json").read_text())
    original = case["request"]
    zero = deepcopy(original)
    zero["messages"][-1]["content"] = json.loads(case["interventions"][0]["replacement"])
    no_call = deepcopy(original)
    del no_call["messages"][2]["tool_calls"]
    control = deepcopy(original)
    control["messages"][-1]["content"]["quantity"] = 17
    requests = {"original": original, "zero": zero, "omit_tool_calls": no_call, "quantity_17": control}
    archives = {"liquid": HERE.parents[1] / "bfcl-transfer/mlx/files.json", "community": HERE / "community-files.json"}
    prompts, models, tokenizers = {}, {}, {}
    report = {"protocol_sha256": digest((HERE / "protocol.md").read_bytes()),
              "versions": {p: version(p) for p in ("mlx", "mlx-lm", "transformers")}, "renderings": {}, "generations": []}
    for name, path in paths.items():
        for filename, expected in json.loads(archives[name].read_text()).items():
            with (path / filename).open("rb") as stream:
                if file_digest(stream, "sha256").hexdigest() != expected:
                    raise ValueError(f"Unregistered {name} checkpoint file: {filename}")
        models[name], tok = load(str(path))
        tokenizers[name] = tok
        for condition, request in requests.items():
            text, ids, _, _ = prompt_input(tok, request["messages"], request["tools"])
            prompts[name, condition] = text
            report["renderings"].setdefault(name, {})[condition] = {
                "prompt_sha256": digest(text.encode()), "input_tokens": len(ids), "ids_sha256": tokens_hash(ids)}
    for name, model in models.items():
        tok = tokenizers[name]
        for author in paths:
            for condition in ("original", "zero"):
                text = prompts[author, condition]
                ids = tok.encode(text, add_special_tokens=tok.bos_token is None or not text.startswith(tok.bos_token))
                output = list(stream_generate(model, tok, ids, max_tokens=original["max_tokens"], sampler=make_sampler(temp=0)))
                assert output[-1].finish_reason == "stop" and output[-1].token in tok.eos_token_ids
                tokens = [event.token for event in output[:-1]]
                answer = "".join(event.text for event in output)
                report["generations"].append({"checkpoint": name, "template": author, "condition": condition,
                    "input_ids_sha256": tokens_hash(ids), "output_tokens": tokens, "streamed_text": answer,
                    "decoded_text": tok.decode(tokens, skip_special_tokens=False), "output_sha256": digest(answer.encode())})
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("liquid", "community", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--repeat", type=Path)
    args = parser.parse_args()
    result = measure({"liquid": args.liquid, "community": args.community})
    if args.repeat:
        if result != json.loads(args.repeat.read_text()):
            raise ValueError("The fresh measurement differs from the original.")
        result = {"matched_generations": len(result["generations"]), "report_sha256": digest(args.repeat.read_bytes())}
    with args.output.open("x") as stream:
        json.dump(result, stream, indent=2, ensure_ascii=False)
        stream.write("\n")
