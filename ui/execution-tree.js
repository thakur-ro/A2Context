"use strict";

function openExecutionTree(runIndex, callIndex) {
  state.view = "timeline";
  state.timelineCall = { run: runIndex, call: callIndex, decision: state.decision };
  element("timeline-layout").value = "execution";
  render();
  focusTimelineCall(state.timelineCall);
}

function focusTimelineCall({ run, call }) {
  const key = run + ":" + call;
  if (!element("timeline-tree").hidden) {
    element("timeline-tree").querySelector(`[data-execution-call="${key}"]`)?.focus();
    return;
  }
  const row = element("timeline-list").querySelector(`[data-timeline-call="${key}"]`);
  if (row) {
    row.tabIndex = -1;
    row.focus();
  }
}

function executionRows(run) {
  const spans = [...(run.spans || [])].sort(compareTimes);
  const byId = new Map(spans.map((span) => [span.id, span]));
  const children = new Map();
  const roots = [];
  for (const span of spans) {
    const parent = span.parent_span_id;
    if (parent && byId.has(parent)) {
      if (!children.has(parent)) children.set(parent, []);
      children.get(parent).push(span);
    } else {
      roots.push(span);
    }
  }

  const rows = [];
  const visited = new Set();
  const walk = (start, issue) => {
    const pending = [{ span: start, depth: 0, issue }];
    while (pending.length) {
      const row = pending.pop();
      if (visited.has(row.span.id)) continue;
      visited.add(row.span.id);
      rows.push(row);
      const nested = children.get(row.span.id) || [];
      for (let index = nested.length - 1; index >= 0; index -= 1) {
        pending.push({ span: nested[index], depth: row.depth + 1 });
      }
    }
  };
  for (const span of roots) {
    walk(span, span.parent_span_id ? "Parent span unavailable" : undefined);
  }
  for (const span of spans) {
    if (!visited.has(span.id)) walk(span, "Recorded parent links form a cycle");
  }
  return { rows, byId };
}

function renderExecutionTree(container, thread) {
  container.replaceChildren();
  for (const runIndex of thread.runs) {
    const run = data.runs[runIndex];
    const trace = node("section", undefined, "panel execution-trace");
    trace.append(node("h2", thread.runs.length > 1 ? run.id : "Recorded spans"));
    const { rows, byId } = executionRows(run);
    if (!rows.length) trace.append(node("p", "Parent spans were unavailable for this trace.", "muted"));
    const callsBySpan = new Map();
    const unmatched = [];
    for (const [callIndex, call] of run.calls.entries()) {
      if (byId.has(call.span_id)) {
        if (!callsBySpan.has(call.span_id)) callsBySpan.set(call.span_id, []);
        callsBySpan.get(call.span_id).push(callIndex);
      } else {
        unmatched.push(callIndex);
      }
    }

    const list = node("ol", undefined, "execution-tree");
    for (const { span, depth, issue } of rows) {
      const row = node("li", undefined, "execution-span");
      row.style.setProperty("--depth", Math.min(depth, 6));
      const body = node("div", undefined, "execution-span-body");
      const calls = callsBySpan.get(span.id);
      if (calls?.length) {
        for (const callIndex of calls) body.append(executionCallButton(runIndex, callIndex, span.name));
      } else {
        body.append(node("strong", span.name || "Recorded span"), node("small", "Span metadata", "anchor"));
      }
      if (issue) body.append(node("small", issue, "execution-span-issue"));
      const provenance = [span.id, span.parent_span_id && "Parent " + span.parent_span_id, span.started_at];
      body.append(node("small", provenance.filter(Boolean).join(" · "), "anchor"));
      row.append(body);
      list.append(row);
    }
    for (const callIndex of unmatched) {
      const row = node("li", undefined, "execution-span");
      row.append(executionCallButton(runIndex, callIndex));
      list.append(row);
    }
    trace.append(list);
    container.append(trace);
  }
}

function executionCallButton(runIndex, callIndex, spanName) {
  const call = data.runs[runIndex].calls[callIndex];
  const title = spanName || call.retrieval?.label || call.decisions.map((decision) => decision.label).join(", ") || "Inspect call";
  const button = node("button", title);
  button.dataset.executionCall = runIndex + ":" + callIndex;
  button.addEventListener("click", () => openDecision(runIndex, callIndex));
  return button;
}
