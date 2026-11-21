"""Generate proposals under the registered arms without dispatching tools."""

from hashlib import sha256
import json
import random
from urllib.error import URLError

from check_proposals import send


def trials(requests, session, call_id, model, summarize):
    for seed in (11,29,47,71,89):
        order = list(requests)
        random.Random(f"{session}/{seed}").shuffle(order)
        row = {"session":session,"call":call_id,"seed":seed,"order":order}
        for arm in order:
            body = dict(requests[arm], seed=seed, temperature=0.5, max_tokens=512)
            try:
                status, data = send(model,"/v1/chat/completions",body,expected=None)
                outcome = {"http":status}
                if status == 200:
                    message = data["choices"][0]["message"]
                    outcome.update(summarize(message))
                    outcome["message_sha256"] = sha256(json.dumps(message,sort_keys=True).encode()).hexdigest()
            except (TimeoutError, URLError) as exc:
                outcome = {"error":str(exc)}
            row[arm] = outcome
        yield row
