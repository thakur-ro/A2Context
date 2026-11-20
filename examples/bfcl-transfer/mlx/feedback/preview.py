"""Build a local Before inference upload and verify its scores with the Rust candidate-probe CLI."""
import argparse
from hashlib import sha256
import json
import math
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

from measure import FEEDBACK, HERE, NAMES
from runner import load_model, trial
from inputs import select
from run import confirm, save


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("upstream","snapshot","binary","output"):
        parser.add_argument("--"+flag, type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Supply an unused path for the locally reconstructed capture.")
    _, packets, schemas, cls = select(args.upstream)
    study = json.loads((HERE/"results.json").read_text())
    design = json.loads((HERE.parent/"results.json").read_text())["design"]
    case = next(p for p in packets if p["id"] == study["id"])
    model, tokenizer = load_model(args.snapshot)
    audit = []
    actual = trial(model, tokenizer, case, schemas, cls, audit=audit)
    confirm(actual, study["arms"]["original"]["trajectory"], tokenizer)
    messages = audit[1]["messages"]
    if messages[-1] != {"role":"tool","content":json.dumps([FEEDBACK["original"]])}:
        raise ValueError("The captured source must be the fare error at the second inference.")
    content = messages[-1]["content"].encode()
    start = content.index(FEEDBACK["original"].encode())
    fragment = {"pointer":f"/messages/{len(messages)-1}/content",
                "text_range":{"start_byte":start,"end_byte":start+len(FEEDBACK["original"].encode())}}
    request = {"version":1,"attributes":{
        "gen_ai.request.model":design["model"],
        "nexusweave.request.body":json.dumps({"model":design["model"],"messages":messages,"tools":schemas},ensure_ascii=False),
        "nexusweave.context.semantics":{"version":1,"request_sources":[{
            "id":"fare-error","source_id":"fare-error","source_version":"generic-v1","request_fragment":fragment}]}}}
    verified = []
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        save(root/"request.json", request)
        for arm in ("native","generic"):
            spec = {"version":1,"id":"fare-feedback-"+arm,"scorer_id":"lfm-mlx-candidates",
                    "assistant_prefix":tokenizer.tool_call_start,
                    "candidates":[{"id":n,"continuation":"["+n+"("} for n in NAMES],
                    "interventions":[{"context_id":"request-source:fare-error","operation":"replace_text_fragment",
                                      **fragment,"replacement":FEEDBACK[arm]}]}
            case_path, report_path = root/(arm+"-case.json"), root/(arm+"-report.json")
            save(case_path,spec)
            scorer = HERE.parents[3]/"scripts/score_mlx_candidates.py"
            subprocess.run([args.binary,"probe-candidates",root/"request.json",case_path,report_path,"--",
                            sys.executable,scorer,args.snapshot,"--model-id",design["model"],"--revision",design["revision"],"--token-steps"],check=True)
            report = json.loads(report_path.read_text())
            for condition, margin, expected in zip(report["scores"]["conditions"],
                    (report["baseline_log_odds"],report["effects"][0]["log_odds"]), ("original",arm), strict=True):
                score = study["arms"][expected]["score"]
                if not math.isclose(margin,score["authentication_minus_fare"],abs_tol=2e-5):
                    raise ValueError("Rust probe margin differs from the checkpoint measurement.")
                edited = [*messages[:-1],{"role":"tool","content":json.dumps([FEEDBACK[expected]])}]
                prompt = tokenizer.apply_chat_template(edited,tools=schemas,tokenize=False,add_generation_prompt=True)
                assert sha256(prompt.encode()).hexdigest() == score["prompt_sha256"]
                assert condition["rendered_prefix_sha256"] == sha256((prompt+spec["assistant_prefix"]).encode()).hexdigest()
            verified.append(report)
    request["attributes"]["nexusweave.candidate_probes"] = verified
    save(args.output,request)
    print(f"Verified original and edited margins. Open {args.output} in Before inference.")


if __name__ == "__main__":
    main()
