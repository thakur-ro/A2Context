"use strict";

const byId = (id) => document.getElementById(id);
const query = new URLSearchParams(location.search);
const view = ["traces", "threads", "preview"].includes(query.get("view")) ? query.get("view") : "findings";
const filterNames = { after: "From", before: "Before", org: "Organization", user: "User",
  workflow: "Workflow", meta_key: "Metadata key", meta_value: "Metadata value" };
const scope = Object.fromEntries(Object.keys(filterNames).filter((key) => query.has(key)).map((key) => [key, query.get(key)]));
const kindLabels = { conflict: "Rule conflict", duplicate: "Exact repetition", replay: "Verified replay",
  candidate: "Candidate scores", shadow: "Proposal trials", token: "Token measurements" };
const recommendationId = query.get("recommendation");
const factorId = query.get("factor");
const traceId = query.get("trace");
const threadId = query.get("thread");
const pageOffset = Number(query.get("offset")) || 0;
const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short", timeZone: "UTC" });

function element(tag, { text, className, ...attributes } = {}, ...children) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  Object.entries(attributes).forEach(([name, value]) => node.setAttribute(name, value));
  node.append(...children.filter(Boolean));
  return node;
}

function link(label, href, className, current = false) {
  return element("a", { text: label, href, className, ...(current && { "aria-current": "page" }) });
}

function url(params, filters = scope) {
  const search = new URLSearchParams(filters);
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === "") search.delete(key);
    else search.set(key, value);
  }
  return search.size ? "?" + search : "/";
}

async function api(path, params = {}) {
  const response = await fetch("/api/sidecar/" + path + "?" + new URLSearchParams(params));
  if (!response.ok) throw new Error((await response.json().catch(() => ({}))).message || "Evidence could not be loaded.");
  return response.json();
}

const plural = (count, name) => count + " " + name + (count === 1 ? "" : "s");
const fullDate = (seconds) => seconds == null ? "Time unavailable" : dateFormat.format(new Date(seconds * 1000)) + " UTC";
const sourceName = (item) => item.source || item.source_id || item.id || "Source ID unavailable";
const version = (value) => value ? " · " + value : "";

function sourceCells(sources, values = []) {
  return element("div", { className: "comparison" }, ...sources.map((source, i) => element("div", {},
    element("strong", { text: sourceName(source) }), element("span", { className: "label", text: source.role || "Role unavailable" }),
    source.version ? element("span", { className: "label", text: " · " + source.version }) : null,
    values[i] === undefined ? null : element("div", {}, element("code", { text: values[i] })))));
}

function trend(rows) {
  if (!rows.length) return element("p", { text: "Capture times unavailable" });
  const max = Math.max(...rows.map((row) => row.count));
  return element("ul", { className: "trend" }, ...rows.slice(0, 7).reverse().map((row) => element("li", {},
    element("time", { text: row.day.slice(5), datetime: row.day }),
    element("meter", { min: 0, max, value: row.count, "aria-label": row.day + " · " + plural(row.count, "observation") }),
    element("span", { text: row.count }))));
}

function replayTrials(detail) {
  const m = detail.measurement;
  const singles = (detail.single_edits || []).map((arm) => ["Edit " + arm.id, arm.summary]);
  const arms = [["Baseline", m.baseline], ...singles, [detail.edit === "joint" ? "Joint edit" : "Source edit", m.variant], ["Control edit", m.control]];
  return element("div", { className: "comparison" }, ...arms.filter(([, data]) => data).map(([label, data]) =>
    element("div", {}, element("strong", { text: label }),
      element("span", { text: data.matched + " / " + data.attempted + " matched" }),
      element("meter", { min: 0, max: Math.max(1, data.attempted), value: data.matched, "aria-label": label + " matched trials out of attempted trials" }),
      data.unscored || data.completed < data.attempted ? element("div", { className: "label", text: plural(data.unscored || 0, "unscored trial") + " · " + data.completed + " completed" }) : null)));
}

const signed = (number) => (number > 0 ? "+" : "") + Number(number.toFixed(3));

function candidateVisual(detail) {
  const alternatives = detail.alternatives.map((entry) => entry.id);
  const readout = (label, margin) => element("div", {},
    element("strong", { text: label }),
    element("span", { text: margin === 0 ? "Tied" : alternatives[margin > 0 ? 0 : 1] + " favored" }),
    element("p", { className: "label", text: signed(margin) + " log odds" }));
  return element("div", { className: "comparison" }, readout("Original request", detail.baseline), readout("One source edited", detail.edited));
}

function tokenVisual(landmark) {
  const max = Math.max(...landmark.positions.map(({ baseline, edited }) => Math.abs(edited - baseline)));
  return element("ol", { className: "trend tokens" }, ...landmark.positions.map(({ token, fragment, baseline, edited }) =>
    element("li", { ...(token === landmark.token && { className: "selected-token" }) },
      element("code", { text: JSON.stringify(fragment), title: "Output token " + (token + 1) }),
      element("meter", { min: 0, max, value: Math.abs(edited - baseline),
        "aria-label": "Output token " + (token + 1) + " · change " + signed(edited - baseline) }),
      element("span", { text: signed(edited - baseline) }))));
}

function visual(kind, detail, rows = []) {
  const [label, body] = kind === "conflict" ? ["Captured declarations", sourceCells(detail.sources, detail.values)] :
    kind === "replay" || kind === "shadow" ? ["Matches by trial arm", replayTrials(detail)] :
    kind === "candidate" ? [detail.alternatives.map((entry) => entry.id).join(" vs ") + " at a fixed prefix", candidateVisual(detail)] :
    kind === "token" ? ["Change in output-token log probability", tokenVisual(detail.landmark)] :
    rows.length ? ["Occurrences by day (UTC)", trend(rows)] : ["Repeated sources", sourceCells(detail.sources)];
  return element("div", { className: "visual" }, element("span", { className: "label", text: label }), body);
}

function findingTitle(kind, detail) {
  if (kind === "conflict") return "Conflicting values for " + detail.rule.replaceAll("_", " ");
  if (kind === "duplicate" && detail.dialogue_only) return "The same dialogue appears at " + plural(detail.copies, "message location");
  if (kind === "duplicate") return "The same input appears " + detail.copies + " times";
  if (kind === "candidate") {
    const id = sourceName(detail.sources[0]);
    if (detail.baseline * detail.edited < 0) return "Editing " + id + " switched the preferred alternative";
    return "Editing " + id + " shifted candidate scores";
  }
  if (kind === "token") return "Editing " + sourceName(detail.sources[0]) + " changed a token score";
  if (detail.joint_only) return "Only the joint edit changed the match rate";
  const amount = Math.abs(detail.effect_pp);
  const points = amount < 0.01 ? "less than 0.01 percentage points" :
    plural(Number(amount.toFixed(2)), "percentage point");
  if (detail.effect_pp === 0) return "Measured match rates were equal";
  if (detail.edit === "control") return "Target and control differed by " + points;
  return "Match rate " + (detail.effect_pp > 0 ? "rose by " : "fell by ") + points;
}

function findingDetail(kind, detail) {
  if (kind === "conflict") return detail.rule + " · " + detail.scope + " scope";
  if (kind === "duplicate") return "Captured input text matches exactly.";
  const sources = detail.sources.map(sourceName).join(" and ");
  if (kind === "candidate") return signed(detail.change) + " change in log odds · " + detail.model;
  if (kind === "token") return "Output token " + (detail.landmark.token + 1) + " of " + detail.landmark.total_tokens + " · " + detail.landmark.model;
  if (kind === "shadow") return detail.outcome.label + " · Proposal-only test of " + sources;
  return detail.outcome + " · " + (detail.edit === "joint" ? "Joint edit of " : "Tested edit of ") + sources;
}

function tag(kind) {
  return element("span", { className: "kind " + kind, text: kindLabels[kind] });
}

function factorName(sources) {
  return sources.map((source) => sourceName(source) + (source.version ? " (" + source.version + ")" : "")).join(" + ");
}

function impactPanel(impact) {
  return element("section", { className: "impact", "aria-label": "Conversation impact" },
    element("div", {}, element("h2", { text: "Conversations with findings" }),
      element("strong", { className: "impact-number", text: impact.conversations }),
      element("p", { text: plural(impact.factors, "source factor") + (impact.unlinked_requests ? " · " + plural(impact.unlinked_requests, "additional request") + " without a conversation ID" : "") })),
    element("div", { className: "impact-breakdown" },
      element("strong", { text: impact.changed + " with a measured change" }),
      element("meter", { min: 0, max: Math.max(1, impact.conversations), value: impact.changed,
        "aria-label": "Conversations with a change under a tested edit" }),
      element("p", { text: "A source edit changed a score or an outcome in a test linked to these conversations." })));
}

function factorRow(factor) {
  const signals = { joint: "Only the joint edit changed the match rate", flip: "Choice flipped", replay: "Replay outcome shifted", shadow: "Proposal rate shifted",
    token: "Token score shifted", score: "Preference shifted", conflict: "Conflicting rules",
    duplicate: "Repeated input", dialogue: "Repeated dialogue", equal: "Equal match rates" };
  return element("a", { className: "factor-row", href: url({ factor: factor.id }) },
    element("div", {}, element("strong", { text: factorName(factor.sources) }),
      element("span", { className: "row-extra", text: [factor.org, plural(factor.conversations, "identified conversation"),
        plural(factor.requests, "request"), plural(factor.recommendations, "recommendation")].filter(Boolean).join(" · ") })),
    element("span", { className: ["conflict", "duplicate", "dialogue"].includes(factor.signal) ? "basis" : "basis measured",
      text: signals[factor.signal] }),
    element("span", { className: "arrow", "aria-hidden": "true", text: "→" }));
}

function findingCard(card) {
  const { kind, evidence: { detail } } = card;
  return element("article", { className: "card" },
    element("div", {}, tag(kind), element("h2", { text: findingTitle(kind, detail) }),
      element("p", { text: findingDetail(kind, detail) }),
      element("p", { className: "label", text: plural(card.occurrences, "observation") + " · " + plural(card.traces, "request") }),
      link("Review recommendation", url({ factor: factorId, recommendation: card.id }), "action")),
    element("div", { className: "suggestion" }, element("span", { className: "label", text: "Next experiment" }),
      element("p", { text: detail.joint_only ? "Test the two sources together" : card.action.title })));
}

function panel(title, ...children) {
  return element("section", { className: "panel" }, element("h2", { text: title }), ...children);
}

function listing(items, render) {
  return element("ul", { className: "list" }, ...items.map((item) => element("li", {}, render(item))));
}

function rowLink(title, subtitle, href, current = false) {
  return element("a", { href, ...(current && { "aria-current": "true" }) },
    element("span", { className: "row-main", text: title }),
    subtitle ? element("span", { className: "row-extra", text: subtitle }) : null);
}

function pagination(next, params = {}) {
  byId("pagination").replaceChildren(...[
    pageOffset > 0 ? link("Previous", url({ view, ...params, offset: Math.max(0, pageOffset - 15) || null })) : null,
    next == null ? null : link("Next", url({ view, ...params, offset: next })),
  ].filter(Boolean));
}

function traceRows(rows) {
  return listing(rows, (item) => rowLink(item.workflow || item.source, [item.id, fullDate(item.start),
    item.thread ? "Thread " + item.thread : "", item.with_inputs + "/" + item.calls + " inference spans with inputs"].filter(Boolean).join(" · "),
  url({ view: "traces", trace: item.id })));
}

function advancedHref(trace, call, item) {
  const m = item?.detail.measurement;
  const hash = new URLSearchParams({ v: "1", view: "decisions", run: trace.id, call: call.id });
  const decision = m?.location?.decision_id || call.decisions?.[0]?.id;
  if (decision) hash.set("decision", decision);
  if (m) {
    hash.set("replay", m.replay_id);
    if (m.context_id) hash.set("variant", m.context_id);
    if (m.control_context_id) hash.set("control", m.control_context_id);
  }
  return "/details?" + new URLSearchParams({ run: trace.id }) + "#" + hash;
}

function prepareEdit(trace, call, item) {
  const source = item.detail.sources?.[0];
  const decision = call.decisions[0];
  if (!source?.id || !decision || !call.context.some((input) => input.id === source.id && input.request_fragment)) return null;
  const hash = new URLSearchParams({ v: "1", view: "decisions", run: trace.id, call: call.id, decision: decision.id, context: source.id });
  return link("Prepare an edit", "/details?" + new URLSearchParams({ run: trace.id }) + "#" + hash, "action");
}

function compareForm(group) {
  const result = element("div", { "aria-live": "polite" });
  const user = element("input", { name: "compare_user", placeholder: "User ID", required: "" });
  const form = element("form", {}, element("label", { text: "Compare with a user in the same filters" }, user),
    element("button", { type: "submit", text: "Compare" }));
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const otherUser = user.value.trim();
    if (!otherUser) {
      result.textContent = "Enter a user ID.";
      return;
    }
    result.textContent = "Loading comparison…";
    try {
      const filters = { ...scope, group, ...(factorId && { factor: factorId }) };
      const [current, other] = await Promise.all([api("findings", filters),
        api("findings", { ...filters, user: otherUser })]);
      const describe = (rows) => rows.length ? plural(rows[0].traces, "trace") + " · " + plural(rows[0].threads, "identified thread") : "No captured matches";
      result.replaceChildren(element("div", { className: "comparison" }, ...[
        [scope.user || "Current scope", current.rows], [otherUser, other.rows],
      ].map(([label, rows]) => element("div", {}, element("strong", { text: label }),
        element("p", { text: describe(rows) }), rows.length ? trend(rows[0].trend) : null))));
    } catch (error) {
      result.textContent = error.message;
    }
  });
  return element("details", { className: "panel" }, element("summary", { text: "Compare recurrence" }), form, result);
}

function recommendationContext(kind, detail) {
  if (kind === "candidate") {
    const other = detail.other_edits.map((edit) => element("li", { text: sourceName(edit) + " · " +
      signed(edit.change) + " change in log odds" }));
    return element("div", {}, element("p", { text: "Both alternatives were scored from the same assistant prefix." }),
      element("pre", { text: detail.prefix || "(Empty assistant prefix)" }),
      other.length ? element("h3", { text: "Other edits in this test" }) : null,
      other.length ? element("ul", {}, ...other) : null);
  }
  if (kind === "token") {
    const point = detail.landmark;
    const selected = point.positions.find((position) => position.token === point.token);
    const percent = new Intl.NumberFormat(undefined, { style: "percent", maximumSignificantDigits: 4 });
    return element("div", {},
      element("p", { text: "For the original token " + JSON.stringify(selected.fragment) + ", the probability changed from " +
        percent.format(Math.exp(selected.baseline)) + " to " + percent.format(Math.exp(selected.edited)) +
        " after editing the source. Both scores use the original output prefix." }),
      element("p", { text: "At this token, the largest tested decoder-block swap was at block " +
        point.block + " (" + signed(point.block_delta) + " log probability)." }));
  }
  if (kind === "replay" && detail.edit === "joint" && detail.single_edits?.length) {
    return element("div", {},
      element("p", { text: "Joint interaction: " + (detail.measurement.interaction_pp == null ? "unavailable" :
        signed(detail.measurement.interaction_pp) + " percentage points beyond the sum of the two individual changes.") }),
      element("details", {}, element("summary", { text: "Tested replacements" }),
        ...detail.single_edits.map((arm) => element("p", { text: arm.id + " · " + (arm.replacement || "(Removed text)") }))));
  }
  if (kind === "shadow") {
    return element("details", {}, element("summary", { text: "Show expected proposal" }),
      element("pre", { text: JSON.stringify(detail.outcome, null, 2) }));
  }
  return null;
}

function recommendationDetail(kind, detail, action, rows = []) {
  const interval = detail.measurement?.interval || detail.measurement?.difference_95_interval_pp;
  const uncertainty = interval ? "95% interval for the difference: " + interval.lower + " to " + interval.upper + " percentage points." :
    ({ replay: "The supplied trials have no estimated interval for this comparison.",
       shadow: "The supplied proposal trials have no estimated interval for this comparison.",
       candidate: "These scores compare two specified alternatives on one request.",
       token: "This comparison holds the output prefix fixed at each position.",
       conflict: "The disagreement is visible in the inputs. Its effect on the answer still needs a test.",
       duplicate: "The text matches exactly. Its effect on the answer still needs a test." })[kind];
  return panel(findingTitle(kind, detail), tag(kind),
    element("p", { className: "label", text: action.basis }), visual(kind, detail, rows),
    recommendationContext(kind, detail), element("p", { className: "uncertainty", text: uncertainty }),
    element("details", {}, element("summary", { text: "How this was measured" }),
      element("p", { text: ({
        candidate: "A model runner scored two fixed continuations before choosing an answer. The sidecar verified that its report matches the captured request and source edit.",
        shadow: "A model runner proposed answers or tool calls for the baseline and edited requests. The sidecar checked the request and recomputed the match rates.",
        token: "An instrumented model runner replaced a source and swapped decoder-block activations on a fixed output path. This view shows where the original token’s score moved under those interventions.",
        replay: "A runner replayed the original request and the specified edits. The sidecar checked the request digests and counted the returned answers or tool calls that matched the recorded outcome.",
        conflict: "The sidecar compared the rule key, scope and declared value on captured inputs. This check can run as soon as the workflow assembles a request.",
        duplicate: "The sidecar found the same text at separate input locations. This check can run before the workflow sends the request."
      })[kind] })),
    element("h3", { text: detail.joint_only ? "Keep one source available while testing removal" : action.title }),
    element("p", { text: detail.joint_only ? "Repeat both single edits and the joint edit with a second replacement. A different wording can change the result." : action.next }),
    detail.replacement == null ? null : element("details", {}, element("summary", { text: "Show tested replacement" }),
      element("pre", { text: detail.replacement || "(Removed text)" })));
}

function review(card, occurrences) {
  const { trace_id, call_id, detail } = card.evidence;
  byId("title").textContent = "Recommendation";
  byId("summary").textContent = card.action.basis;
  const traceLink = url({ view: "traces", trace: trace_id, call: call_id, finding: card.id });
  const evidence = element("details", { className: "panel" }, element("summary", { text: "Underlying evidence" }),
    link("Open original span", traceLink),
    card.kind === "replay" ? link("Open trial comparison", advancedHref({ id: trace_id }, { id: call_id }, { detail })) : null);
  return element("div", { className: "feed" },
    link("Back to source factor", url({ factor: factorId || detail.factor })),
    recommendationDetail(card.kind, detail, card.action, card.trend),
    card.threads > 1 ? compareForm(card.id) : null, evidence,
    panel("Where this appeared", ...occurrences.rows.map((row) => element("details", { className: "occurrence" },
      element("summary", { text: (row.thread || "Request without a conversation ID") + " · " + fullDate(row.observed) }),
      element("p", { text: findingTitle(row.kind, row.detail) }), visual(row.kind, row.detail),
      element("div", { className: "actions" },
        row.thread_key ? link("Open conversation", url({ view: "threads", thread: row.thread_key })) : null,
        link("Open span " + row.call, url({ view: "traces", trace: row.trace, call: row.call, finding: card.id })))))));
}

function pendingPreview() {
  const output = element("section", { className: "feed", "aria-live": "polite" });
  const input = element("input", { type: "file", accept: "application/json,.json" });
  async function show(load) {
    output.textContent = "Checking the request…";
    try {
      const report = await load();
      output.replaceChildren(
        element("p", { className: "label", text: plural(report.captured_inputs, "captured input") +
          " · " + plural(report.total, "recommendation") }),
        ...report.unverified_probes.map((issue) => element("p", { className: "empty", text: issue })),
        ...report.recommendations.map(({ kind, detail, action }) => recommendationDetail(kind, detail, action)),
        report.total ? element("p", { className: "label", text: report.total > 15 ? "Showing the first 15 recommendations." : "" }) :
          element("p", { className: "empty", text: "No supported issues or measured shifts in this request." }));
    } catch (error) {
      output.textContent = error.message;
    }
  }
  input.addEventListener("change", () => {
    const file = input.files[0];
    if (!file) return;
    show(async () => {
      if (file.size > 2 * 1024 * 1024) throw new Error("Choose a JSON request under 2 MiB.");
      const submitted = JSON.parse(await file.text());
      const envelope = submitted.attributes ? { request: submitted } : submitted;
      const response = await fetch("/api/sidecar/preview", { method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify(envelope) });
      const result = await response.json();
      if (!response.ok) throw new Error(result.message);
      return result;
    });
  });
  const policy = element("button", { type: "button", text: "Try a policy conflict" });
  policy.addEventListener("click", () => show(() => api("preview-example", { scenario: "policy" })));
  const routing = element("button", { type: "button", text: "Try a measured tool choice" });
  routing.addEventListener("click", () => show(() => api("preview-example", { scenario: "routing" })));
  return element("div", { className: "feed" }, panel("Review an assembled request",
    element("label", { text: "Load a preflight request or a request bundled with probe results" }, input),
    element("div", { className: "actions" }, policy, routing)), output);
}

function traceDetail({ trace, findings, activation_reports }) {
  const selectedCall = query.get("call");
  const selectedSpan = query.get("span");
  const spans = new Map((trace.spans || []).map((span) => [span.id, span]));
  const calls = new Map(trace.calls.map((call) => [call.span_id || call.id, call]));
  const children = new Map();
  for (const span of spans.values()) {
    const parent = spans.has(span.parent_span_id) ? span.parent_span_id : null;
    if (!children.has(parent)) children.set(parent, []);
    children.get(parent).push(span);
  }
  const initial = selectedCall || selectedSpan || trace.calls[0]?.id;
  const call = trace.calls.find((entry) => entry.id === initial || entry.span_id === initial);
  const span = spans.get(call?.span_id || initial);
  const seen = new Set();
  const rows = [];
  function visit(sibling, depth) {
    if (seen.has(sibling.id)) return;
    seen.add(sibling.id);
    const entry = calls.get(sibling.id);
    rows.push({ span: sibling, entry, depth });
    for (const child of children.get(sibling.id) || []) visit(child, depth + 1);
  }
  for (const root of children.get(null) || []) visit(root, 0);
  for (const rest of spans.values()) visit(rest, 0);
  for (const [id, entry] of calls) if (!seen.has(id)) rows.push({ span: { id }, entry, depth: 0 });

  byId("title").textContent = "Trace";
  const thread = JSON.stringify([trace.source, trace.organization_id, trace.user_id, trace.conversation_id]);
  byId("summary").replaceChildren(trace.id, ...(trace.conversation_id ?
    [" · ", link(trace.conversation_id, url({ view: "threads", thread }))] : []));
  const matching = findings.filter((item) => item.call_id === call?.id);
  const focus = matching.find((item) => item.group === query.get("finding"));
  const detailItems = focus ? [focus, ...matching.filter((item) => item !== focus)] : matching;
  const evidence = detailItems.map((item) => panel(findingTitle(item.kind, item.detail),
    tag(item.kind), element("p", { text: findingDetail(item.kind, item.detail) }), visual(item.kind, item.detail),
    element("p", { className: "label", text: "Evidence at " + (call.span_id || call.id) }),
    item.kind === "replay" ? link("Open trial comparison", advancedHref(trace, call, item), "action") :
      item.kind === "conflict" || item.kind === "duplicate" ? prepareEdit(trace, call, item) : null));
  const capture = call?.context.filter((item) => item.availability === "included") || [];
  const unavailable = call ? call.context.length - capture.length : 0;
  const inputs = (call?.context || []).map((item) => element("details", {}, element("summary", { text: sourceName(item) + version(item.source_version) }),
    element("p", { className: "label", text: item.evidence.field + (item.evidence.message_index == null ? "" : " · message " + item.evidence.message_index) }),
    item.text == null ? element("p", { text: item.availability === "redacted" ? "Input was redacted" : "Input text unavailable" }) : element("pre", { text: item.text })));
  const detail = panel(call?.model || span?.name || (call ? "Inference span" : "Span"),
    element("p", { className: "label", text: span?.id || call?.id || "No recorded span selected" }),
    call ? element("p", { text: capture.length ? plural(capture.length, "captured input") : "Captured inputs unavailable" }) : null,
    unavailable ? element("p", { className: "label", text: plural(unavailable, "input") + " missing or redacted" }) : null,
    ...inputs,
    ...(call?.decisions || []).map((decision) => element("details", {},
      element("summary", { text: decision.label }), element("pre", { text: JSON.stringify(decision.outcome, null, 2) }))),
    call ? link(activation_reports ? "Open detailed investigation and token measurements" :
      "Open detailed investigation", advancedHref(trace, call)) : null);
  return element("div", { className: "detail" }, panel("Span tree", listing(rows, ({ span: row, entry, depth }) =>
    rowLink("↳ ".repeat(Math.min(depth, 6)) + (row.name || entry?.model || row.id),
      entry ? (entry.turn_id ? "Turn " + entry.turn_id : "Captured call") : "Parent span",
      url({ view, trace: trace.id, ...(entry ? { call: entry.id } : { span: row.id }) }), entry?.id === initial || row.id === initial))),
  element("div", { className: "feed" }, ...evidence, focus ? compareForm(focus.group) : null, detail));
}

async function render(refresh = false) {
  let body;
  if (view === "traces" && traceId) {
    body = traceDetail(await api("trace", { id: traceId }));
  } else if (view === "threads" && threadId) {
    const data = await api("traces", { ...scope, thread: threadId, offset: pageOffset });
    byId("title").textContent = data.rows[0]?.thread || "Thread";
    byId("summary").textContent = "Traces in chronological order.";
    body = data.rows.length ? traceRows(data.rows) : null;
    pagination(data.next_offset, { thread: threadId });
  } else if (view === "findings") {
    if (recommendationId) {
      const filters = { ...scope, group: recommendationId, ...(factorId && { factor: factorId }) };
      const [cards, occurrences] = await Promise.all([api("findings", filters),
        api("occurrences", { ...filters, offset: pageOffset })]);
      if (cards.rows.length) body = review(cards.rows[0], occurrences);
      pagination(occurrences.next_offset, { recommendation: recommendationId, factor: factorId });
    } else if (factorId) {
      const data = await api("findings", { ...scope, factor: factorId, offset: pageOffset });
      byId("title").textContent = data.rows.length ? factorName(data.rows[0].evidence.detail.sources) : "Source factor";
      byId("summary").textContent = "Each recommendation below uses its own comparison and outcome.";
      body = element("div", { className: "feed" }, link("All source factors", url({})),
        ...data.rows.map(findingCard), !data.rows.length ? element("p", { className: "empty", text: "No evidence in this scope." }) : null);
      pagination(data.next_offset, { factor: factorId });
    } else {
      const [impact, factors] = await Promise.all([api("impact", scope), api("factors", { ...scope, offset: pageOffset })]);
      const signature = JSON.stringify([impact, factors]);
      if (refresh && byId("content").dataset.signature === signature) return;
      byId("content").dataset.signature = signature;
      body = element("div", { className: "overview" }, impactPanel(impact),
        element("section", { className: "factors" }, element("h2", { text: "Source factors" }),
          factors.rows.length ? listing(factors.rows, factorRow) :
            element("p", { className: "empty", text: "No recommendations in this scope." })));
      pagination(factors.next_offset);
    }
  } else {
    const data = await api(view, { ...scope, offset: pageOffset });
    if (data.rows.length) body = view === "traces" ? traceRows(data.rows) : listing(data.rows, (item) => rowLink(item.thread,
      [item.org, item.user, plural(item.trace_count, "trace"), fullDate(item.start)].filter(Boolean).join(" · "), url({ view, thread: item.id })));
    pagination(data.next_offset);
  }
  byId("content").replaceChildren(body || element("p", { className: "empty", text: view === "findings" ?
    "No recommendations in this scope. Cards appear as supported evidence arrives." : "No captured records in this scope." }));
}

async function status() {
  const health = await api("health", { ...scope, ...(threadId && { thread: threadId }) });
  const { coverage } = health;
  byId("environment").textContent = health.demo ? "Demo workspace" : "Live workspace";
  byId("status").textContent = [
    health.demo ? "Saved demo evidence" : plural(coverage.traces, "captured trace"),
    coverage.with_inputs + "/" + coverage.inference_spans + " inference spans with inputs",
    coverage.with_rules + "/" + coverage.inference_spans + " with declared rules",
    coverage.with_request + "/" + coverage.inference_spans + " with replayable request captured",
    health.queue_drops ? plural(health.queue_drops, "queue drop") : null,
  ].filter(Boolean).join(" · ");
  return health;
}

async function main() {
  const hash = new URLSearchParams(location.hash.slice(1));
  if (hash.get("v") === "1" && ["conversation", "decisions", "timeline", "patterns", "compare", "pending"].includes(hash.get("view"))) {
    const params = new URLSearchParams();
    if (hash.has("a_run")) {
      params.set("run", hash.get("a_run"));
      if (hash.has("b_run")) params.set("other", hash.get("b_run"));
    } else if (hash.has("thread")) params.set("thread", hash.get("thread"));
    else if (hash.has("run")) params.set("run", hash.get("run"));
    else params.set("legacy", "1");
    location.replace("/details?" + params + location.hash);
    return;
  }
  const views = { findings: "Recommendations", preview: "Before inference", traces: "Traces", threads: "Threads" };
  byId("navigation").replaceChildren(...Object.entries(views).filter(([key]) => ["findings", "preview"].includes(key)).map(([key, label]) => link(label, url({ view: key }), null, view === key || (key === "findings" && view !== "preview"))));
  byId("title").textContent = views[view];
  byId("summary").textContent = { findings: "See which conversations warrant a closer look, then choose a source to investigate.",
    preview: "Check an assembled request and any supplied tests before committing a decision.",
    traces: "Open a trace to inspect its spans and captured sources.",
    threads: "Browse traces grouped by their recorded conversation." }[view];
  if (view === "preview") {
    byId("filters").hidden = true;
    byId("capture").hidden = true;
    byId("content").replaceChildren(pendingPreview());
    return;
  }
  const form = byId("filter-form");
  byId("reset").href = url({ view }, {});
  byId("filter-count").textContent = Object.keys(scope).length ? " · " + Object.keys(scope).length : "";
  byId("scope").replaceChildren(...Object.entries(scope).map(([key, value]) => element("span", { className: "chip", text: filterNames[key] + " · " + value })));
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const values = Object.fromEntries([...new FormData(form)].filter(([, value]) => value.trim()).map(([key, value]) => [key, value.trim()]));
    if (!values.meta_key || !values.meta_value) {
      delete values.meta_key;
      delete values.meta_value;
    }
    location.href = url({ view, trace: traceId, thread: threadId, call: query.get("call"),
      span: query.get("span"), finding: query.get("finding"), factor: factorId, recommendation: recommendationId }, values);
  });
  const health = await status();
  const meta = form.elements.namedItem("meta_key");
  for (const key of health.metadata_keys) meta.add(element("option", { value: key, text: key }));
  if (scope.meta_key && !health.metadata_keys.includes(scope.meta_key)) meta.add(element("option", { value: scope.meta_key, text: scope.meta_key }));
  for (const [key, value] of Object.entries(scope)) form.elements.namedItem(key).value = value;
  await render();
  if (view === "findings" && !factorId && !recommendationId && !health.demo) window.setInterval(() => {
    if (!document.hidden && !byId("content").contains(document.activeElement)) {
      Promise.all([status(), render(true)]).catch((error) => { byId("error").textContent = error.message; });
    }
  }, 15000);
}

main().catch((error) => { byId("error").textContent = error.message; });
