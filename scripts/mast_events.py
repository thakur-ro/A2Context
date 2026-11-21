"""Extract observable transcript events from the pinned MAST log formats."""

import ast
from dataclasses import dataclass
from io import StringIO
import re
import tokenize


@dataclass(frozen=True)
class Event:
    actor: str
    channel: str
    start: int
    end: int
    text: str


def ag2(body):
    if not body.lstrip().startswith("{"):
        blocks = re.finditer(r"(?ms)^ {6}content:[ \t]*(?P<text>.*?)^ {6}role:[^\n]*\n {6}name: (?P<actor>[^\n]+)", body)
        return [Event(m["actor"], "message", *m.span("text"), m["text"].strip()) for m in blocks]

    # The export concatenates Python literals on one line. Tokenize braces outside strings.
    if "\n" in body.rstrip():
        return []
    events = []
    depth = 0
    start = 0
    try:
        for token in tokenize.generate_tokens(StringIO(body).readline):
            if token.type != tokenize.OP:
                continue
            if token.string == "{":
                if depth == 0:
                    start = token.start[1]
                depth += 1
            elif token.string == "}":
                depth -= 1
                if depth == 0:
                    end = token.end[1]
                    record = ast.literal_eval(body[start:end])
                    if not isinstance(record, dict):
                        return []
                    content = record.get("content")
                    if isinstance(content, list) and all(isinstance(line, str) for line in content):
                        content = "\n".join(content)
                    if isinstance(content, str) and isinstance(record.get("name"), str):
                        events.append(Event(record["name"], "message", start, end, content.strip()))
    except (ValueError, SyntaxError, tokenize.TokenError):
        return []
    return events


def marked_events(body, pattern, channel):
    markers = list(re.finditer(pattern, body, re.M))
    ends = [match.start() for match in markers[1:]] + [len(body)]
    events = []
    for match, end in zip(markers, ends):
        actor = match["actor"]
        if actor and actor.casefold() not in {"user", "system", "human"}:
            start = match.end()
            events.append(Event(actor.strip(), channel, start, end, body[start:end].strip()))
    return events


def chatdev(body):
    # ChatDev logs the other actor's bracketed system message before the response.
    headers = list(re.finditer(r"(?m)^\[\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} [A-Z]+\] (?P<header>[^\n]*)", body))
    ends = [match.start() for match in headers[1:]] + [len(body)]
    events = []
    for match, end in zip(headers, ends):
        role = re.fullmatch(r"(?P<actor>[^:]+): \*\*.*<->.* on\s*:\s*(?P<phase>[^,]+), turn \d+\*\*", match["header"])
        if not role:
            continue
        boundary = body.find("]\n\n", match.end(), end)
        start = boundary + 3
        if boundary >= 0 and body[match.end():boundary].lstrip().startswith("["):
            events.append(Event(role["actor"], role["phase"].strip(), start, end, body[start:end].strip()))
    return events


def hyperagent(body):
    pattern = r"(?ms)^\s*Intern Name:[ \t]+(?P<actor>[^\n]+)|^[ \t]*Action:[ \t]*\n[ \t]*```\w*[ \t]*\n(?P<text>.*?)^[ \t]*```[ \t]*$"
    actor = None
    events = []
    for match in re.finditer(pattern, body):
        if match["actor"]:
            actor = match["actor"].strip()
        elif actor:
            events.append(Event(actor, "action", *match.span("text"), match["text"].strip()))
    return events


def openmanus(body):
    pattern = r"(?m)^.+\| app\.agent\.toolcall:think:\d+ - .+Tools being prepared: \[['\"](?P<tool>[\w-]+)['\"]\]\s*\n.+\| app\.agent\.toolcall:think:\d+ - .+Tool arguments: (?P<text>[^\n]+)$"
    return [Event("Manus", m["tool"], *m.span("text"), m["text"].strip()) for m in re.finditer(pattern, body)]


def events_for(framework, body):
    if framework == "AG2":
        return ag2(body)
    if framework == "ChatDev":
        return chatdev(body)
    if framework == "HyperAgent":
        return hyperagent(body)
    if framework == "OpenManus":
        return openmanus(body)
    if framework in {"Magentic", "GAIA"}:
        return marked_events(body, r"^---------- (?P<actor>[\w-]+) ----------[ \t]*$", "message")
    if framework == "AppWorld":
        return marked_events(body, r"^[ \t]*(?:Response from (?P<actor>[\w -]+) Agent|Message to .+ Agent|Code Execution Output|(?:Entering|Exiting) .+ Agent message loop)[ \t]*$", "response")
    if framework == "MetaGPT":
        return marked_events(body, r"^(?:(?P<actor>SimpleCoder|SimpleReviewer|SimpleTester):[ \t]*|[-]{20,}|\[[^\]\n]+\] (?:NEW MESSAGES:|FROM:.*))[ \t]*$", "message")
    return []
