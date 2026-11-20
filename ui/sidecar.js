"use strict";

const byId = (id) => document.getElementById(id);
const query = new URLSearchParams(location.search);
const view = ["traces", "threads"].includes(query.get("view")) ? query.get("view") : "findings";
const filterNames = { after: "From", before: "Before", org: "Organization", user: "User",
  workflow: "Workflow", meta_key: "Metadata key", meta_value: "Metadata value" };
const scope = Object.fromEntries(Object.keys(filterNames).filter((key) => query.has(key)).map((key) => [key, query.get(key)]));
const kindLabels = { conflict: "Rule conflict", duplicate: "Exact repetition", replay: "Verified replay" };
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
  const arms = [["Baseline", m.baseline], [detail.edit === "joint" ? "Joint edit" : "Source edit", m.variant], ["Control edit", m.control]];
  return element("div", { className: "comparison" }, ...arms.filter(([, data]) => data).map(([label, data]) =>
    element("div", {}, element("strong", { text: label }),
      element("span", { text: data.matched + " / " + data.attempted + " matched" }),
      data.unscored || data.completed < data.attempted ? element("div", { className: "label", text: plural(data.unscored || 0, "unscored trial") + " · " + data.completed + " completed" }) : null)));
}

function visual(kind, detail, rows = []) {
  const [label, body] = kind === "conflict" ? ["Captured declarations", sourceCells(detail.sources, detail.values)] :
    kind === "replay" ? ["Matches by trial arm", replayTrials(detail)] :
    rows.length ? ["Occurrences by day (UTC)", trend(rows)] : ["Repeated sources", sourceCells(detail.sources)];
  return element("div", { className: "visual" }, element("span", { className: "label", text: label }), body);
}

function findingTitle(kind, detail) {
  if (kind === "conflict") return "Conflicting values for " + detail.rule.replaceAll("_", " ");
  if (kind === "duplicate") return "The same input appears " + detail.copies + " times";
  const amount = Math.abs(Number(detail.effect_pp.toFixed(2)));
  const points = plural(amount, "percentage point");
  if (detail.effect_pp === 0) return "Measured match rates were equal";
  if (detail.edit === "control") return "Target and control differed by " + points;
  return "Match rate " + (detail.effect_pp > 0 ? "rose " : "fell ") + points;
}

function findingDetail(kind, detail) {
  if (kind === "conflict") return detail.rule + " · " + detail.scope + " scope";
  if (kind === "duplicate") return "Captured input text matches exactly.";
  const sources = detail.sources.map(sourceName).join(" and ");
  return detail.outcome + " · " + (detail.edit === "joint" ? "Joint edit of " : "Tested edit of ") + sources;
}

function tag(kind) {
  return element("span", { className: "kind " + kind, text: kindLabels[kind] });
}

function findingCard(card) {
  const { kind, evidence } = card;
  const { detail } = evidence;
  const href = url({ view: "traces", trace: evidence.trace_id, call: evidence.call_id, finding: card.id });
  return element("article", { className: "card" }, element("div", {}, tag(kind),
    element("h2", { text: findingTitle(kind, detail) }),
    element("p", { text: findingDetail(kind, detail) }),
    element("p", { text: [plural(card.occurrences, "observation"), plural(card.traces, "trace"),
      card.threads ? plural(card.threads, "identified thread") : "Thread IDs unavailable"].join(" · ") }),
    card.latest == null ? null : element("p", { className: "label", text: "Last seen " + fullDate(card.latest) }),
    link("Open evidence", href, "action")), visual(kind, detail, card.trend));
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
  const decision = m?.location.decision_id || call.decisions[0]?.id;
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
    result.textContent = "Loading comparison…";
    try {
      const [current, other] = await Promise.all([api("findings", { ...scope, group }),
        api("findings", { ...scope, user: user.value.trim(), group })]);
      const describe = (rows) => rows.length ? plural(rows[0].traces, "trace") + " · " + plural(rows[0].threads, "identified thread") : "No captured matches";
      result.replaceChildren(element("div", { className: "comparison" }, ...[
        [scope.user || "Current scope", current.rows], [user.value.trim(), other.rows],
      ].map(([label, rows]) => element("div", {}, element("strong", { text: label }),
        element("p", { text: describe(rows) }), rows.length ? trend(rows[0].trend) : null))));
    } catch (error) {
      result.textContent = error.message;
    }
  });
  return element("details", { className: "panel" }, element("summary", { text: "Compare recurrence" }), form, result);
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
    item.kind === "replay" ? link("Open trial comparison", advancedHref(trace, call, item), "action") : prepareEdit(trace, call, item)));
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
  } else {
    const data = await api(view, { ...scope, offset: pageOffset });
    if (data.rows.length) {
      if (view === "findings") {
        const signature = JSON.stringify(data);
        if (refresh && byId("content").dataset.signature === signature) return;
        byId("content").dataset.signature = signature;
        body = element("div", { className: "feed" }, ...data.rows.map(findingCard));
      } else if (view === "traces") body = traceRows(data.rows);
      else body = listing(data.rows, (item) => rowLink(item.thread,
        [item.org, item.user, plural(item.trace_count, "trace"), fullDate(item.start)].filter(Boolean).join(" · "),
        url({ view, thread: item.id })));
    }
    pagination(data.next_offset);
  }
  byId("content").replaceChildren(body || element("p", { className: "empty", text: view === "findings" ?
    "No findings in this scope. Findings appear as supported evidence arrives." : "No captured records in this scope." }));
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
  const views = { findings: "Findings", traces: "Traces", threads: "Threads" };
  byId("navigation").replaceChildren(...Object.entries(views).map(([key, label]) => link(label, url({ view: key }), null, view === key)));
  byId("title").textContent = views[view];
  byId("summary").textContent = { findings: "Investigate context conflicts, repetition, and tested edits.",
    traces: "Open a trace to inspect its spans and captured sources.",
    threads: "Browse traces grouped by their recorded conversation." }[view];
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
      span: query.get("span"), finding: query.get("finding") }, values);
  });
  const health = await status();
  const meta = form.elements.namedItem("meta_key");
  for (const key of health.metadata_keys) meta.add(element("option", { value: key, text: key }));
  if (scope.meta_key && !health.metadata_keys.includes(scope.meta_key)) meta.add(element("option", { value: scope.meta_key, text: scope.meta_key }));
  for (const [key, value] of Object.entries(scope)) form.elements.namedItem(key).value = value;
  await render();
  if (view === "findings" && !health.demo) window.setInterval(() => {
    if (!document.hidden && !byId("content").contains(document.activeElement)) {
      Promise.all([status(), render(true)]).catch((error) => { byId("status").textContent = error.message; });
    }
  }, 15000);
}

main().catch((error) => { byId("status").textContent = error.message; });
