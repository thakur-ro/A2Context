"""Render the per-arm order states from the bounded retry experiment (Matplotlib 3.10.8)."""
import argparse
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import Patch, Rectangle

HERE = Path(__file__).resolve().parent
ARMS = {"original": "Original", "cache": "Cache", "record": "Authorization\nrecord",
        "customer": "Customer\nmessage", "control": "Unrelated\nrecord"}


def plot(output):
    design = json.loads((HERE / "design.json").read_text())
    results = json.loads((HERE / "framework/retry-results.json").read_text())
    trials = {(r["case"], r["arm"]): r for r in results["rows"]}
    cases = design["cases"]
    target = design["pending_arguments"]["order_id"]
    colors = {True: "#e4edf3", False: "#f7ddce"}
    fig, ax = plt.subplots(figsize=(10.2, 5.1))
    fig.subplots_adjust(left=.25, right=.985, top=.77, bottom=.10)
    fig.text(.025, .93, "What happened after each source edit?", weight="bold", fontsize=17)
    fig.text(.025, .866, "Eight exchange requests, one retry after an argument rejection, approval gate enabled.", fontsize=11)
    names = []
    for y, case in enumerate(cases):
        intent, channel, wording = case["id"].split("-")
        names.append(f"{intent.capitalize()}  ·  {channel.capitalize()} {wording.upper()}")
        for x, arm in enumerate(ARMS):
            trial = trials[case["id"], arm]
            receipt = trial["with_gate"]["after"]
            matches = receipt["orders"][target] == trial["expected_state"]
            outcome = receipt["dispatch"] if trial["final_disposition"] == "executed" else trial["final_disposition"]
            label = {"text":"reply", "written":"wrote order"}[outcome]
            label = "Retry → " + label if trial["retry"] else label.capitalize()
            ax.add_patch(Rectangle((x-.49,y-.48),.98,.96,facecolor=colors[matches],edgecolor="none"))
            ax.text(x,y,label,ha="center",va="center",fontsize=10,color="#1c3041")
    ax.set(xlim=(-.5,len(ARMS)-.5),ylim=(len(cases)-.5,-.5))
    ax.set_xticks(range(len(ARMS)),ARMS.values())
    ax.set_yticks(range(len(cases)),names)
    ax.xaxis.tick_top()
    ax.tick_params(length=0,pad=5,labelsize=10)
    ax.spines[:].set_visible(False)
    fig.legend(handles=[Patch(color=colors[True],label="Expected order state"),
                        Patch(color=colors[False],label="Unexpected order state")],
               loc="lower center", ncols=2,frameon=False)
    fig.savefig(output,dpi=180,facecolor="white")
    plt.close(fig)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output",type=Path)
    plot(parser.parse_args().output)
