"""Plot the tool-feedback intervention using only the public measurement receipt."""
import argparse
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.ticker import PercentFormatter

HERE = Path(__file__).resolve().parent


def plot(output):
    arms = json.loads((HERE/"results.json").read_text())["arms"]
    fig = plt.figure(figsize=(9.5,4.0), layout="constrained")
    grid = fig.add_gridspec(2,2,height_ratios=[1,2])
    path = fig.add_subplot(grid[0,:])
    path.set(xlim=(-.42,2.42), ylim=(-.3,1))
    path.axis("off")
    steps = next(iter(arms.values()))["callback_states"]
    names = {"get_flight_cost":"Fare lookup","authenticate_travel":"Authentication","compute_exchange_rate":"Exchange rate"}
    for i, (call,) in enumerate(steps):
        if i:
            path.annotate("",(i-.06,.68),(i-1+.06,.68),arrowprops={"arrowstyle":"->","color":"#9da7b2"})
        changed = call["before"] != call["after"]
        path.scatter(i,.68,color="#845632" if changed else "#4c6178",s=75)
        path.text(i,.36,names[call["name"]],ha="center",size=11,weight="semibold")
        path.text(i,.07,"Public state changed" if changed else "Public state held",ha="center",size=9,color="#4b5563")
    labels = ["Original error","Native diagnostic","Generic paraphrase"]
    for col, (key,title) in enumerate([("opening_probability","Probability the next token opens a tool call"),
                                      ("authentication_minus_fare","Authentication vs. fare log odds")]):
        ax = fig.add_subplot(grid[1,col])
        values = [arms[k]["score"][key] for k in ("original","native","generic")]
        ax.scatter(values,[2,1,0],color=["#555e68","#395c8b","#8995a1"],s=52)
        ax.set(ylim=(-.5,2.62),yticks=[2,1,0],yticklabels=labels if col==0 else [])
        ax.set_title(title,fontsize=11,loc="left",pad=8)
        ax.set_axisbelow(True)
        ax.grid(axis="x",color="#e2e5e9")
        ax.spines[:].set_visible(False)
        ax.tick_params(length=0,labelsize=9)
        if col==0:
            ax.set_xlim(0,1); ax.xaxis.set_major_formatter(PercentFormatter(1,decimals=0))
        else:
            ax.set_xlim(0,max(values)+2); ax.set_xlabel("Natural log odds given the tool-call opener",fontsize=9)
        for y,v in zip([2,1,0],values):
            ax.annotate(f"{v:.1%}" if col==0 else f"{v:+.2f}",(v,y),xytext=(8,0),textcoords="offset points",va="center",size=9)
    fig.suptitle("Three error messages led to the same three tool calls",fontsize=16,weight="semibold")
    fig.savefig(output,dpi=150,facecolor="white")
    plt.close(fig)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output",type=Path)
    plot(parser.parse_args().output)
