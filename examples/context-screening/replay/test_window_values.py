"""Change only the stated return window and replay two saved questions on local Qwen."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

FIXTURES = Path(__file__).resolve().parent
ROOT = FIXTURES.parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path, help="Pinned local Qwen2.5-0.5B-Instruct checkpoint")
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/nexusweave")
    args = parser.parse_args()
    args.output.mkdir(exist_ok=True, parents=True)
    with TemporaryDirectory() as folder:
        for question, filename in [("earlier","case.json"), ("followup","case-followup.json")]:
            case = json.loads((FIXTURES / filename).read_text())
            case["id"] += "-45-days"
            for edit, original in zip(case["interventions"], ("30", "60"), strict=True):
                index = int(edit["pointer"].split("/")[2])
                content = case["request"]["messages"][index]["content"]
                assert content.count(original + " days") == 1
                edit["replacement"] = content.replace(original + " days", "45 days")
            path = Path(folder) / filename
            path.write_text(json.dumps(case))
            subprocess.run([str(args.binary), "replay", str(FIXTURES / "evidence.json"), str(path),
                            str(args.output / (question + "-45-days.json")), "--", sys.executable,
                            str(ROOT / "scripts/propose_qwen.py"), str(args.model),
                            "--model-id", case["request"]["model"]], check=True)


if __name__ == "__main__":
    main()
