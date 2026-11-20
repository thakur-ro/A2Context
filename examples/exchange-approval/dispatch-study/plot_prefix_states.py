"""Plot paired changes in expected order state from the saved prefix experiment."""
import argparse
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt


def plot(output):
    source = Path(__file__).parent / "prefill-tool/results.json"
    report = json.loads(source.read_text())
    fig, ax = plt.subplots(figsize=(8.2, 2.7))
    fig.subplots_adjust(left=.24, right=.97, bottom=.20, top=.73)
    fig.text(.03, .89, "What changed when the tool-call opener was supplied?", size=15, weight="bold")
    fig.text(.03, .77, "Expected order state, compared with free decoding of the same request", size=10)
    bound = len(report["pairs"])
    ax.set_xlim(-bound, bound)
    ax.set_ylim(-.6, 1.6)
    ax.axvline(0, color="#687789", linewidth=1)
    for y, key in enumerate(["with_gate", "without_gate"]):
        changes = report["summary"]["paired"][key]
        for outcome, sign, color in [("lost", -1, "#94543c"), ("restored", 1, "#39597e")]:
            count = changes.get(outcome, 0)
            if count:
                ax.barh(y, sign * count, height=.55, color=color)
                ax.text(sign * count / 2, y, f"{count} {outcome}", ha="center", va="center", color="white", size=10)
        other = [f"{n} {label}" for label, n in changes.items() if label not in ("lost", "restored") and n]
        if other:
            ax.text(changes.get("restored", 0)+.22, y, ", ".join(other), va="center", size=9)
    ax.set_yticks([1, 0], ["Without approval check", "With approval check"])
    ax.set_xticks([])
    ax.tick_params(length=0)
    ax.spines[:].set_visible(False)
    fig.text(.03, .08, f"{bound} paired requests across {len(set(row['case'] for row in report['pairs']))} synthetic orders. Each order used two cache settings.", size=9, color="#475569")
    fig.savefig(output, dpi=180, facecolor="white")
    plt.close(fig)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    plot(parser.parse_args().output)
