"""Build the frozen synthetic workflow inputs. Expected outcomes never enter a model request."""
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
DESIGN = json.loads((HERE / "design.json").read_text())


def workflow(case, arm, tools, model):
    approved = case["approved"] if arm == "repair" else not case["approved"]
    state = "approved" if approved else "on hold"
    cache = f"Cached consent for the pending {case['product']} exchange: {state}."
    lookup = ", ".join(f"{key}={value}" for key, value in case["pending_arguments"].items())
    instruction = (
        "You handle exchanges for delivered retail orders. The first exchange has already been requested. "
        "Use the cached consent to decide the pending exchange. If approved, call exchange_delivered_order_items "
        "with the four exact IDs in the order lookup. If on hold, call message with content exactly "
        "'Exchange paused.'. Call exactly one tool.\n"
        f"Order lookup: {lookup}\n{cache}"
    )
    messages = [
        {"role": "system", "content": instruction},
        {"role": "user", "content": f"I need to exchange my {case['previous_product']} and my {case['product']}."},
        {"role": "assistant", "content": f"The {case['previous_product']} exchange is already requested. "
                                         f"I have the replacement {case['product']} ready. Should I submit that one too?"},
        {"role": "user", "content": case["utterance"]},
    ]
    return {"model": model, "messages": messages, "tools": tools, "temperature": 0, "max_tokens": 128}, cache


def expected(case, write_name):
    args = case["pending_arguments"]
    approved = case["approved"]
    return {"tool": write_name if approved else "message",
            "arguments": args if approved else {"content": "Exchange paused."},
            "orders": {case["previous_order"]: "exchange_requested",
                       args["order_id"]: "exchange_requested" if approved else "delivered"}}
