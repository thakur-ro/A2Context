"""Select first-turn checkpoints and replay literal reference calls in BFCL's in-memory TravelAPI."""
import argparse
import ast
from copy import deepcopy
from hashlib import sha256
import json
from pathlib import Path
import sys
from tempfile import NamedTemporaryFile
from urllib.request import urlopen

HERE = Path(__file__).resolve().parent
FILES = json.loads((HERE / "sources.json").read_text())
BASE = Path("berkeley-function-call-leaderboard/bfcl_eval")


def digest(value):
    return sha256(json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def fetch(root, hashes=None):
    for name, expected in (FILES["sha256"] if hashes is None else hashes).items():
        target = root / name
        if target.exists():
            if sha256(target.read_bytes()).hexdigest() != expected:
                raise ValueError(f"Existing local copy differs for {name}")
            continue
        url = f"https://raw.githubusercontent.com/ShishirPatil/gorilla/{FILES['revision']}/{name}"
        with urlopen(url, timeout=90) as response:
            raw = response.read()
        if sha256(raw).hexdigest() != expected:
            raise ValueError(f"Download differs for {name}")
        target.parent.mkdir(parents=True, exist_ok=True)
        with NamedTemporaryFile(dir=target.parent, delete=False) as tmp:
            tmp.write(raw)
        Path(tmp.name).replace(target)


def public_state(sim):
    return {k: v for k, v in vars(sim).items() if not k.startswith("_")}


def new_sim(cls, state):
    sim = cls()
    sim._load_scenario(deepcopy(state))
    return sim


def calls(expressions, names):
    result = []
    for expression in expressions:
        node = ast.parse(expression, mode="eval").body
        if (not isinstance(node, ast.Call) or not isinstance(node.func, ast.Name)
                or node.func.id not in names or node.args or any(kw.arg is None for kw in node.keywords)):
            raise ValueError("Reference must be a named TravelAPI call with explicit literal keyword arguments.")
        args = {kw.arg: ast.literal_eval(kw.value) for kw in node.keywords}
        result.append((node.func.id, args))
    return result


def select(root):
    for name, expected in FILES["sha256"].items():
        if sha256((root / name).read_bytes()).hexdigest() != expected:
            raise ValueError(f"Changed upstream artifact {name}")
    base = root / BASE
    # A sparse checkout can omit these four empty namespace initializers.
    for parent in (base, base / "eval_checker", base / "eval_checker/multi_turn_eval",
                   base / "eval_checker/multi_turn_eval/func_source_code"):
        init = parent / "__init__.py"
        if init.exists() and init.read_bytes():
            raise ValueError(f"Unregistered upstream package initializer {init}")
    sys.path.insert(0, str(base.parent))
    from bfcl_eval.eval_checker.multi_turn_eval.func_source_code.travel_booking import TravelAPI

    def rows(name):
        return [json.loads(line) for line in (base / "data" / name).read_text().splitlines()]
    tasks = rows("BFCL_v4_multi_turn_base.json")
    references = {r["id"]: r["ground_truth"][0] for r in rows("possible_answer/BFCL_v4_multi_turn_base.json")}
    schemas = rows("multi_turn_func_doc/travel_booking.json")
    for schema in schemas:
        if schema["parameters"]["type"] != "dict":
            raise ValueError("Unexpected source schema")
        schema["parameters"]["type"] = "object"
        for field in schema["parameters"]["properties"].values():
            if field["type"] == "float":
                field["type"] = "number"
    names = {s["name"] for s in schemas}
    if len(names) != 18 or not all(hasattr(TravelAPI, n) and not n.startswith("_") for n in names):
        raise ValueError("Unexpected travel tool surface.")
    wanted, packets, locations = {"write": 3, "read": 3}, [], []
    seen = set()
    counts = {"base_rows": len(tasks), "travel_only": 0, "calendar_excluded": 0}
    for task in tasks:
        if task["involved_classes"] != ["TravelAPI"]:
            continue
        counts["travel_only"] += 1
        tid = task["id"]
        ref = calls(references[tid], names)
        if any(n == "verify_traveler_information" for n, _ in ref):
            counts["calendar_excluded"] += 1
            continue
        state = task["initial_config"]["TravelAPI"]
        first = new_sim(TravelAPI, state)
        final = new_sim(TravelAPI, state)
        outputs = [getattr(final, n)(**args) for n, args in ref]
        stratum = "read" if first == final else "write"
        signature = digest(public_state(first))
        if not wanted[stratum] or signature in seen:
            continue
        if any(isinstance(o, dict) and "error" in o for o in outputs):
            raise ValueError("Selected reference returned an error.")
        wanted[stratum] -= 1
        seen.add(signature)
        messages = task["question"][0]
        if len(messages) != 1 or messages[0]["role"] != "user" or task.get("excluded_function"):
            raise ValueError("Expected one first user message and all travel tools.")
        locations.append({"id": tid, "stratum": stratum, "first_turn_sha256": digest(messages),
                          "initial_state_sha256": signature, "reference_state_sha256": digest(public_state(final)),
                          "reference_calls": [{"name": n, "arguments_sha256": digest(a)} for n, a in ref],
                          "reference_responses_sha256": [digest(r) for r in outputs]})
        packets.append({"id": tid, "messages": messages, "state": state, "ref": ref, "outputs": outputs})
    if any(wanted.values()):
        raise ValueError("Unable to fill the reference-defined strata.")
    meta = {"revision": FILES["revision"], "protocol_sha256": sha256((HERE / "protocol.md").read_bytes()).hexdigest(),
            "sources_sha256": sha256((HERE / "sources.json").read_bytes()).hexdigest(), "census": counts,
            "effective_tools_sha256": digest(schemas), "cases": locations}
    return meta, packets, schemas, TravelAPI


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("checkout", type=Path)
    p.add_argument("--fetch", action="store_true", help="Download only the eight pinned upstream files.")
    args = p.parse_args()
    if args.fetch:
        fetch(args.checkout)
    print(json.dumps(select(args.checkout)[0], indent=2))
