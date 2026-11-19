"use strict";

const capturesByCall = new Map();
const captureLabels = {
  available: "Available",
  redacted: "Redacted",
  reference_only: "Reference only",
  unavailable: "Unavailable",
};

function captureOnCall(runId, callId) {
  return capturesByCall.get(JSON.stringify([runId, callId]));
}

function indexCaptureProfiles() {
  const runs = new Map(data.runs.map((run) => [run.id, run]));
  for (const profile of data.capture_profiles || []) {
    const run = runs.get(profile.run_id);
    const call = run.calls.find((item) => item.id === profile.call_id);
    const entries = new Map(profile.items.map((item) => [item.context_id, item]));
    const counts = { available: 0, redacted: 0, reference_only: 0, unavailable: 0, unmatched: 0, single: 0, multiple: 0 };
    for (const item of entries.values()) {
      counts[item.capture] += 1;
      if (item.matching_fields === undefined) continue;
      const status = item.matching_fields === 0 ? "unmatched" : item.matching_fields === 1 ? "single" : "multiple";
      counts[status] += 1;
    }
    const name = call.decisions.map((decision) => decision.label).join(", ") || call.model || call.id;
    const thread = [run.organization_id, run.user_id, run.conversation_id || run.id].filter(Boolean).join(" / ");
    const search = [
      thread, run.id, call.id, name, call.model, run.source,
      ...call.context.flatMap((item) => [item.source_id, item.kind, item.role, item.document_identity?.id]),
    ].filter(Boolean).join(" ").toLowerCase();
    const sourceGaps = call.context.flatMap((item) => {
      if (!item.source_id) return [];
      const capture = entries.get(item.id);
      const status = capture.capture !== "available" ? capture.capture :
        capture.matching_fields === 0 ? "unmatched" : capture.matching_fields > 1 ? "multiple" : null;
      return status ? [{ source: item.source_id, version: item.source_version, status, contextId: item.id }] : [];
    });
    capturesByCall.set(JSON.stringify([profile.run_id, profile.call_id]), {
      ...profile, entries, counts, search, name, thread, sourceGaps,
      organization_id: run.organization_id, user_id: run.user_id, conversation_id: run.conversation_id,
      trace_source: run.source, started_at: call.started_at,
    });
  }
}

function captureInventory({ items, counts }) {
  if (!items.length) return "No context entries recorded";
  return Object.entries(captureLabels).flatMap(([key, label]) =>
    counts[key] ? [counts[key] + " " + label.toLowerCase()] : []
  ).join(" · ");
}

function requestMatchInventory({ request_evidence: request, counts }) {
  if (!request) return "Matching fields unknown";
  if (!counts.available) return "No available entries to match";
  return [
    counts.single && counts.single + " with one field",
    counts.multiple && counts.multiple + " with multiple fields",
    counts.unmatched && counts.unmatched + " without a match",
  ].filter(Boolean).join(" · ");
}

function renderCaptureSummary(container, profile, full = false) {
  if (!profile) return;
  const summary = node("div", undefined, "capture-summary");
  if (full) summary.id = "capture-on-call";
  summary.append(node("p", captureInventory(profile)));
  const hasRequest = Boolean(profile.request_evidence);
  const request = hasRequest ? "Original request captured" : "Original request unavailable";
  const gaps = hasRequest && profile.counts.available ? " · " + requestMatchInventory(profile) : "";
  summary.append(node("p", request + gaps));
  if (full && hasRequest) summary.append(evidence(profile.request_evidence));
  if (full) summary.append(evidenceLinkButton({ capture_profile: true }, "capture coverage for this call"));
  if (full) container.prepend(summary);
  else container.append(summary);
}

function renderItemCapture(details, item, requestFragment) {
  if (!item) return;
  const fields = item.matching_fields;
  if (fields !== undefined) {
    const label = requestFragment
      ? fields ? "Declared fragment matches the captured request" : "Declared fragment did not match the captured request"
      : fields ? countLabel(fields, "matching request field") : "No exact field in the captured request";
    details.append(node("small", label, "anchor"));
  } else if (item.capture === "reference_only") {
    details.append(node("small", "Only a content reference was captured", "anchor"));
  }
}

function renderPromptSourceLayout(container, call, capture) {
  if (!capture?.request_evidence) return;
  const fields = new Map();
  for (const item of call.context) {
    const fragment = item.request_fragment;
    if (!fragment || capture.entries.get(item.id)?.matching_fields !== 1) continue;
    const { pointer, text_range: range } = fragment;
    if (!fields.has(pointer)) fields.set(pointer, []);
    fields.get(pointer).push({ item, start: range.start_byte, end: range.end_byte });
  }
  if (!fields.size) return;
  const details = node("details", undefined, "prompt-source-layout");
  details.append(node("summary", "Prompt source layout"));
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    for (const [pointer, fragments] of fields) {
      details.append(promptSourceField(pointer, fragments));
    }
  });
  container.prepend(details);
}

function promptSourceField(pointer, fragments) {
  const field = node("section", undefined, "prompt-source-field");
  const extent = fragments.reduce((maximum, { end }) => Math.max(maximum, end), 0);
  field.append(node("h3", pointer), node("small", "0 to " + extent + " UTF-8 bytes · Last declared offset", "anchor"));
  fragments.sort((a, b) => a.start - b.start || b.end - a.end);
  let earlierEnd = 0;
  for (const [index, { item, start, end }] of fragments.entries()) {
    const overlaps = start < earlierEnd || (index + 1 < fragments.length && fragments[index + 1].start < end);
    earlierEnd = Math.max(earlierEnd, end);
    const entry = node("div", undefined, "prompt-source-range");
    const label = [item.source_id || item.id, item.source_version].filter(Boolean).join(" · ");
    const button = node("button", label);
    button.type = "button";
    button.dataset.fragmentId = item.id;
    button.addEventListener("click", () => openDecision(state.run, state.call, state.decision, { context_item_id: item.id }));
    const offsets = node("small", "Bytes " + start + " to " + end + " (end excluded)" +
      (overlaps ? " · Overlaps another range" : ""), "anchor");
    const track = node("div", undefined, "prompt-source-track");
    track.setAttribute("aria-hidden", "true");
    const bar = node("span", undefined, overlaps ? "overlapping" : undefined);
    bar.style.left = (100 * start / extent) + "%";
    bar.style.width = (100 * (end - start) / extent) + "%";
    track.append(bar);
    entry.append(button, offsets, track);
    field.append(entry);
  }
  return field;
}

function matchesCaptureStatus(profile, status) {
  switch (status) {
    case "request": return !profile.request_evidence;
    case "empty": return !profile.items.length;
    case "unavailable": return profile.items.length > profile.counts.available;
    case "unmatched": return profile.counts.unmatched > 0;
    case "multiple": return profile.counts.multiple > 0;
    default: return true;
  }
}

function renderCapturePatterns(list, organization, user, query) {
  const groups = scopedPatternGroups([{ occurrences: [...capturesByCall.values()] }], organization, user);
  updatePatternThreadFilter(groups);
  const status = element("capture-status").value;
  const selected = (groups[0]?.occurrences || []).filter((item) =>
    (!state.patternThread || patternThreadKey(item) === state.patternThread) && item.search.includes(query)
  );
  if (!selected.length) {
    list.append(node("p", "No calls with capture records match this scope and filter.", "muted"));
    return;
  }
  const captured = selected.filter((item) => item.request_evidence).length;
  const refs = selected.filter(({ counts, items }) => counts.available < items.length).length;
  const matched = selected.filter(({ counts }) => counts.single || counts.multiple).length;
  const panel = node("article", undefined, "panel capture-pattern");
  panel.append(node("h2", "Recorded input coverage"));
  panel.append(node("p", [
    captured + " of " + selected.length + " calls with a captured request",
    countLabel(matched, "call") + " with exact field matches",
    countLabel(refs, "call") + " with unavailable entries",
  ].join(" · "), "count"));
  const rows = selected.filter((item) => matchesCaptureStatus(item, status)).sort(compareTimes);
  if (rows.length) {
    appendRecurringGaps(panel, rows, status);
    panel.append(captureTable(rows));
  }
  else panel.append(node("p", "No calls have this capture status in the selected scope.", "muted"));
  list.append(panel);
}

function appendRecurringGaps(panel, rows, filter) {
  const groups = new Map();
  for (const profile of rows) {
    for (const gap of profile.sourceGaps) {
      const hidden = gap.status !== "unmatched" && gap.status !== "multiple";
      if (filter && filter !== "request" && filter !== gap.status && !(filter === "unavailable" && hidden)) continue;
      const key = JSON.stringify([gap.source, gap.version, gap.status]);
      if (!groups.has(key)) groups.set(key, { gap, calls: new Map() });
      groups.get(key).calls.set(JSON.stringify([profile.run_id, profile.call_id]), { profile, id: gap.contextId });
    }
  }
  const recurring = [...groups.values()].filter(({ calls }) => calls.size > 1)
    .sort((left, right) => right.calls.size - left.calls.size);
  if (!recurring.length) return;
  const details = node("details", undefined, "capture-gaps");
  details.append(node("summary", "Recurring gaps on named sources"));
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    for (const { gap, calls } of recurring) {
      const entry = node("details");
      const label = captureLabels[gap.status] || (gap.status === "multiple" ? "Multiple matching fields" : "No matching field");
      entry.append(node("summary", [gap.source, gap.version, label + " on " + countLabel(calls.size, "call")].filter(Boolean).join(" · ")));
      entry.addEventListener("toggle", () => {
        if (!entry.open || entry.childElementCount > 1) return;
        for (const { profile, id } of calls.values()) {
          entry.append(occurrenceButton(profile, profile.thread + " · " + profile.name, { context_item_id: id }));
        }
      });
      details.append(entry);
    }
  });
  panel.append(details);
}

function captureTable(profiles) {
  const table = node("table", undefined, "effects-table capture-table");
  const heading = table.createTHead().insertRow();
  for (const title of ["Call", "Context entries", "Original request", "Exact request matches"]) {
    const label = node("th", title);
    label.scope = "col";
    heading.append(label);
  }
  const body = table.createTBody();
  const more = node("button", "Show more calls");
  const appendPage = () => {
    for (const item of profiles.slice(body.rows.length, body.rows.length + 25)) {
      const row = body.insertRow();
      const call = row.insertCell();
      call.append(occurrenceButton(item, item.thread + " · " + item.name, { capture_profile: true }));
      call.append(node("small", [item.started_at, item.trace_source, item.call_id].filter(Boolean).join(" · "), "anchor"));
      row.insertCell().textContent = captureInventory(item);
      row.insertCell().textContent = item.request_evidence ? "Captured" : "Unavailable";
      row.insertCell().textContent = requestMatchInventory(item);
    }
    more.hidden = body.rows.length === profiles.length;
    more.textContent = "Show more · " + body.rows.length + " of " + profiles.length + " calls shown";
  };
  more.addEventListener("click", appendPage);
  appendPage();
  const scroll = node("div", undefined, "effects-scroll");
  const container = node("div", undefined, "capture-results");
  scroll.append(table);
  container.append(scroll, more);
  return container;
}
