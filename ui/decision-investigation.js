"use strict";

const investigationRequests = {
  ready: "Exact request match",
  choose_field: "Choose a matching field",
  need_request: "Original request unavailable",
  no_match: "No exact request match",
  uncaptured: "Input text unavailable",
  outcome_unavailable: "Replay outcome unavailable",
  tool_execution: "Recorded tool execution",
};

function replayRequestStatus(decision, item, capture) {
  if (decision.kind === "tool_execution") return "tool_execution";
  if (decision.outcome === null) return "outcome_unavailable";
  if (item.availability !== "included" || item.text === null) return "uncaptured";
  if (!capture?.request_evidence) return "need_request";
  const fields = capture.entries.get(item.id)?.matching_fields;
  if (!fields) return "no_match";
  return fields > 1 ? "choose_field" : "ready";
}

function readyToPlanReplay(request) {
  return request === "ready" || request === "choose_field";
}

function investigationInputs(call, decision, capture, exactLinks, findings, replayFor, masksFor = () => []) {
  const signals = new Map();
  function add(id, signal) {
    if (!signals.has(id)) signals.set(id, []);
    signals.get(id).push(signal);
  }
  for (const { link, index } of exactLinks) {
    add(link.context_id, { link, reference: { evidence: { kind: "exact_match", index } } });
  }
  for (const [index, link] of call.relationships.entries()) {
    if (link.decision_id !== decision.id) continue;
    add(link.context_id, { link: { ...link, kind: "annotation" }, reference: { evidence: { kind: "annotation", index } } });
  }
  for (const finding of findings) {
    for (const id of finding.context_ids) add(id, { finding, reference: { finding_index: finding.finding_index } });
  }
  return call.context.map((item) => {
    const request = replayRequestStatus(decision, item, capture);
    const captured = capture?.entries.get(item.id);
    return { item, request, captureStatus: captured?.capture, signals: signals.get(item.id) || [],
      replay: replayFor(item.id), masks: masksFor(item.id) };
  });
}

const investigationFilters = [
  ["evidence", "Linked, flagged, or tested", ({ signals, replay, masks }) => signals.length || replay.effects.length || masks.length],
  ["ready", "Ready to plan", ({ request }) => readyToPlanReplay(request)],
  ["measured", "Measured inputs", ({ replay, masks }) => replay.status === "measured" || masks.length],
  ["all", "All recorded inputs", () => true],
];

function exactDecisionLinks(run, call, decision) {
  return data.context_links.flatMap((link, index) =>
    link.run_id === run.id && link.call_id === call.id && link.decision_id === decision.id ? [{ link, index }] : []);
}

function renderDecisionInvestigation(outcome, run, call, decision, capture, exactLinks) {
  if (!call.context.length) return;
  const { run: runIndex, call: callIndex, decision: decisionIndex } = state;
  const actions = {
    inspect: (reference) => openDecision(runIndex, callIndex, decisionIndex, reference),
    plan: (id) => openReplayPlanner(run, call, decision, [id]),
  };
  const findings = callFindings(run.id, call.id, decision.id);
  const location = { run_id: run.id, call_id: call.id, decision_id: decision.id };
  const inputs = investigationInputs(call, decision, capture, exactLinks, findings,
    (context_id) => contextReplay({ ...location, context_id }),
    (context_id) => masksByDecisionInput.get(contextReplayKey({ ...location, context_id })) || []);
  const groups = investigationFilters.map(([key, label, eligible]) => ({ key, label, items: inputs.filter(eligible) }));
  const measured = inputs.filter(({ replay }) => replay.status === "measured").length;
  const masked = inputs.filter(({ masks }) => masks.length).length;
  const flagged = inputs.filter(({ signals }) => signals.length).length;
  const panel = node("details", undefined, "decision-investigation");
  const counts = [countLabel(inputs.length, "recorded input")];
  if (flagged) counts.push(flagged + " linked or flagged");
  if (measured) counts.push(measured + " with a measured replay");
  if (masked) counts.push(masked + " with a source mask");
  if (groups[1].items.length) counts.push(groups[1].items.length + " ready to plan");
  panel.append(node("summary", "Investigate context · " + counts.join(" · ")));
  panel.addEventListener("toggle", () => {
    if (!panel.open || panel.childElementCount > 1) return;
    const tools = node("div", undefined, "investigation-tools");
    const filter = node("label", "Show inputs ");
    const select = node("select");
    for (const { key, label, items } of groups) select.add(new Option(label + " (" + items.length + ")", key));
    select.value = masked ? "measured" : groups[0].items.length ? "evidence" : "all";
    filter.append(select);
    const searchLabel = node("label", "Search inputs ");
    const search = node("input");
    search.type = "search";
    search.placeholder = "Source or captured text";
    searchLabel.append(search);
    tools.append(filter, searchLabel);
    const results = node("div");
    const renderRows = () => {
      const { items } = groups.find(({ key }) => key === select.value);
      const query = search.value.trim().toLowerCase();
      const matches = query ? items.filter(({ item }) =>
        [item.source_id, item.id, item.role, item.kind, item.text].some((value) => value?.toLowerCase().includes(query))
      ) : items;
      results.replaceChildren(matches.length ? investigationTable(matches, actions, masked > 0)
        : node("p", "No inputs match this filter.", "muted"));
    };
    select.addEventListener("change", renderRows);
    search.addEventListener("input", renderRows);
    renderRows();
    const explanation = masked
      ? "Mask and pair links open the largest absolute change measured in each report. Replay links open the tested edit."
      : "Use links and findings to choose an input. Replay measures the effect of a specific edit.";
    panel.append(node("p", explanation, "muted"), tools, results);
  });
  outcome.append(panel);
}

function investigationTable(inputs, actions, hasMasks) {
  const scroll = node("div", undefined, "effects-scroll");
  const table = node("table", undefined, "effects-table investigation-table");
  // Narrow panels stack the cells, so preserve the table roles explicitly.
  table.setAttribute("role", "table");
  const head = table.createTHead();
  head.setAttribute("role", "rowgroup");
  const heading = head.insertRow();
  heading.setAttribute("role", "row");
  for (const label of ["Recorded input", "Decision evidence", "Measurements and replay"]) {
    const header = node("th", label);
    header.scope = "col";
    header.setAttribute("role", "columnheader");
    heading.append(header);
  }
  const rows = table.createTBody();
  rows.setAttribute("role", "rowgroup");
  const more = node("button", "Show more inputs");
  const appendPage = () => {
    for (const input of inputs.slice(rows.rows.length, rows.rows.length + 12)) {
      const { item, signals, request, replay, masks, captureStatus } = input;
      const row = rows.insertRow();
      row.dataset.inputId = item.id;
      row.setAttribute("role", "row");
      const source = node("th");
      source.scope = "row";
      source.setAttribute("role", "rowheader");
      const name = [item.source_id || item.id, item.source_version].filter(Boolean).join(" · ");
      source.append(investigationButton(name, () => actions.inspect({ context_item_id: item.id })));
      source.append(node("small", [item.role, item.kind.replaceAll("_", " ")].filter(Boolean).join(" · "), "anchor"));
      row.append(source);
      const reasons = row.insertCell();
      reasons.setAttribute("role", "cell");
      for (const { link, finding, reference } of signals) {
        const label = finding ? labels[finding.kind] + " · " + finding.key : contextRelationshipName(link);
        reasons.append(investigationButton(label, () => actions.inspect(reference)));
      }
      if (!signals.length) reasons.textContent = "No recorded link or finding";
      const trial = row.insertCell();
      trial.setAttribute("role", "cell");
      appendSourceMaskEvidence(trial, masks);
      if (hasMasks && !masks.length) trial.append(node("p", "Source mask unavailable", "muted"));
      const status = request === "uncaptured" ? captureLabels[captureStatus] || investigationRequests[request]
        : investigationRequests[request];
      trial.append(node("p", status, "muted"));
      if (replay.effects.length) {
        const label = replay.status === "measured" ? "Open measured replay results" : "Open saved replay trials";
        appendContextReplayResults(trial, replay, label);
      }
      if (["ready", "choose_field", "need_request"].includes(request)) {
        const label = request === "need_request" ? "Plan with a saved request" : "Plan replay";
        trial.append(investigationButton(label, () => actions.plan(item.id)));
      }
    }
    more.hidden = rows.rows.length === inputs.length;
    more.textContent = "Show more · " + rows.rows.length + " of " + inputs.length + " inputs shown";
  };
  more.addEventListener("click", appendPage);
  appendPage();
  scroll.append(table);
  const results = node("div");
  results.append(scroll, more);
  return results;
}

function investigationButton(label, click) {
  const button = node("button", label);
  button.type = "button";
  button.addEventListener("click", click);
  return button;
}
