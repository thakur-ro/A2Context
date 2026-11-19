"use strict";

const comparison = [null, null];
const compareSides = ["a", "b"];
const comparisonStatuses = {
  changed: "Changed evidence",
  incomplete: "Evidence incomplete",
  ambiguous: "Multiple possible matches",
  first: "Recorded only on A",
  second: "Recorded only on B",
  matching: "Matching captured context",
};
const comparisonStatusOrder = Object.keys(comparisonStatuses);
let comparisonInitialized = false;
let pendingComparison;
const comparisonOutcomeLabels = {
  different_kind_or_label: "Different decision kinds or labels",
  unavailable: "Recorded outcome comparison unavailable",
  same: "Same recorded outcome",
  different: "Different recorded outcomes",
};

function selectedDecision() {
  const { run, call, decision } = state;
  return data.runs[run]?.calls[call]?.decisions[decision] ? { run, call, decision } : null;
}

function openComparison() {
  comparison[0] = selectedDecision();
  comparisonInitialized = true;
  state.view = "compare";
  render();
  element("compare-b-run").focus();
}

function compareTransition({ before, after }, decisions = [0, 0]) {
  for (const [index, side] of [before, after].entries()) {
    comparison[index] = { run: side.runIndex, call: side.index, decision: decisions[index] };
  }
  comparisonInitialized = true;
  state.view = "compare";
  element("compare-filter").value = "differences";
  linkNotice();
  render();
  element("compare-outcome-status").tabIndex = -1;
  element("compare-outcome-status").focus();
}

function comparisonSessionLabel(run) {
  return [run.organization_id, run.user_id, run.conversation_id || run.id,
    run.conversation_id && run.id !== run.conversation_id ? run.id : null].filter(Boolean).join(" / ");
}

function comparisonOptions(select, entries, selection, placeholder) {
  const blank = placeholder ? [new Option(placeholder, "")] : [];
  select.replaceChildren(...blank, ...entries.map(([index, title]) => new Option(title, index)));
  select.value = selection ?? "";
  select.disabled = entries.length === 0 || (!placeholder && entries.length < 2);
}

function comparisonCalls(run) {
  return run?.calls.flatMap((call, index) => call.decisions.length ? [[index,
    (index + 1) + " · " + call.decisions.map((decision) => decision.label).join(", ")
  ]] : []) || [];
}

function renderComparison() {
  pendingComparison?.abort();
  if (!comparisonInitialized) {
    comparison[0] = selectedDecision();
    comparisonInitialized = true;
  }
  const runs = data.runs.flatMap((run, index) =>
    run.calls.some((call) => call.decisions.length) ? [[index, comparisonSessionLabel(run)]] : []
  );
  for (const [side, name] of compareSides.entries()) {
    const position = comparison[side];
    const run = data.runs[position?.run];
    const call = run?.calls[position.call];
    comparisonOptions(element("compare-" + name + "-run"), runs, position?.run, "Choose a session");
    comparisonOptions(element("compare-" + name + "-call"), comparisonCalls(run), position?.call);
    comparisonOptions(element("compare-" + name + "-decision"), call?.decisions.map((decision, index) => [
      index, decision.label + " · " + (labels[decision.kind] || decision.kind),
    ]) || [], position?.decision);
  }
  element("compare-controls").onchange = ({ target }) => {
    linkNotice();
    const side = Number(target.dataset.side);
    if (target.value === "") {
      comparison[side] = null;
    } else {
      const value = Number(target.value);
      if (target.dataset.field === "run") {
        comparison[side] = { run: value, call: comparisonCalls(data.runs[value])[0][0], decision: 0 };
      } else if (target.dataset.field === "call") {
        comparison[side] = { ...comparison[side], call: value, decision: 0 };
      } else {
        comparison[side].decision = value;
      }
    }
    renderComparison();
  };

  const sides = comparison.map((position) => {
    if (!position) return null;
    const run = data.runs[position.run];
    const call = run.calls[position.call];
    const decision = call.decisions[position.decision];
    return { run, call, decision, position, location: { run_id: run.id, call_id: call.id, decision_id: decision.id } };
  });
  const ready = sides.every(Boolean)
    && ["run", "call", "decision"].some((field) => comparison[0][field] !== comparison[1][field]);
  element("compare-outcomes").replaceChildren(...sides.flatMap((side, index) =>
    side ? [comparisonOutcome(side, compareSides[index].toUpperCase(), ready)] : []
  ));
  const section = element("compare-context");
  section.hidden = !ready;
  section.setAttribute("aria-busy", String(ready));
  const status = element("compare-outcome-status");
  element("compare-context-rows").replaceChildren();
  element("compare-context-count").textContent = "";
  element("compare-more").hidden = true;
  element("compare-filter").onchange = null;
  if (!ready) {
    status.textContent = runs.length ? "Choose two different decisions to compare." : "No recorded decisions available for comparison.";
    return;
  }
  status.textContent = "Comparing recorded decisions…";
  const controller = new AbortController();
  pendingComparison = controller;
  loadComparison(sides, controller);
}

async function loadComparison(sides, controller) {
  const params = new URLSearchParams();
  for (const [index, prefix] of compareSides.entries()) {
    writeLocation(params, sides[index].position, prefix + "_");
  }
  try {
    const response = await fetch("/api/context-comparison?" + params, { signal: controller.signal });
    const result = await response.json();
    if (controller.signal.aborted || state.view !== "compare") return;
    if (!response.ok) throw new Error(result.message);
    const positions = sides.map(({ location }) => contextCallIndex.get(contextCallKey(location)));
    const ruleText = result.rule_text.map((items) => new Map(items.map(({ context_id, text }) => [context_id, text])));
    const rows = result.context.map((alignment) => {
      const row = resolveContextAlignment(alignment, positions);
      if (row.status === "capture_changed") row.status = "incomplete";
      row.items = row.items.map((items, side) => items.map((item) => ({
        item, ruleText: ruleText[side].get(item.id), replay: contextReplay({ ...sides[side].location, context_id: item.id }),
      })));
      return row;
    }).sort((left, right) => comparisonStatusOrder.indexOf(left.status) - comparisonStatusOrder.indexOf(right.status));
    for (const [index, side] of compareSides.entries()) {
      const body = element("compare-outcomes").querySelector(`[data-compare-body="${side}"]`);
      if (body) body.textContent = result.outcome_text[index] ?? "Outcome body unavailable";
    }
    element("compare-outcome-status").textContent = comparisonOutcomeLabels[result.outcome_status];
    element("compare-filter").onchange = () => renderComparisonRows(rows, sides);
    renderComparisonRows(rows, sides);
  } catch (error) {
    if (controller.signal.aborted || state.view !== "compare") return;
    element("compare-outcome-status").textContent = "Could not compare the selected decisions.";
    for (const body of element("compare-outcomes").querySelectorAll("[data-compare-body]")) {
      body.textContent = "Outcome unavailable for this comparison.";
    }
    element("compare-context-rows").append(node("p", error.message, "muted"));
  } finally {
    if (pendingComparison === controller) element("compare-context").setAttribute("aria-busy", "false");
  }
}

function comparisonOutcome(side, label, waiting) {
  const { run, call, decision, location, position } = side;
  const panel = node("article", undefined, "panel");
  panel.append(node("h2", label + " · " + decision.label));
  panel.append(node("p", [labels[decision.kind] || decision.kind, call.model, run.source, call.started_at].filter(Boolean).join(" · "), "muted"));
  if (decision.outcome === null) panel.append(node("p", "Outcome body unavailable", "muted"));
  else {
    const body = node("pre", waiting ? "Loading recorded outcome…" : readable(decision.outcome));
    body.dataset.compareBody = label.toLowerCase();
    panel.append(body);
  }
  panel.append(evidence(decision.evidence));
  renderCaptureSummary(panel, captureOnCall(location.run_id, location.call_id));
  const findings = callFindings(location.run_id, location.call_id, location.decision_id);
  if (findings.length) {
    const details = node("details", undefined, "comparison-findings");
    details.append(node("summary", findingCounts(findings).join(" · "), "count"));
    for (const item of findings) details.append(findingLink(item, labels[item.kind] + " · " + item.key + " · " + item.scope));
    panel.append(details);
  }
  const open = node("button", "Open decision " + label);
  open.addEventListener("click", () => openDecision(position.run, position.call, position.decision));
  panel.append(open);
  return panel;
}

function renderComparisonRows(rows, sides) {
  const filter = element("compare-filter").value;
  const selected = rows.filter((row) => {
    if (filter === "differences") return row.status !== "matching";
    if (filter === "replay") return row.items.some((items) => items.some(({ replay }) => replay.effects.length));
    return true;
  });
  const container = element("compare-context-rows");
  const more = element("compare-more");
  container.replaceChildren();
  more.hidden = selected.length <= 40;
  element("compare-context-count").textContent = selected.length + " of " + rows.length + " context groups";
  if (!selected.length) {
    container.append(node("p", "No context groups match this filter.", "muted"));
    return;
  }
  let shown = 0;
  const appendPage = () => {
    for (const row of selected.slice(shown, shown + 40)) container.append(comparisonRow(row, sides));
    shown = container.children.length;
    more.disabled = shown === selected.length;
    more.textContent = (more.disabled ? "" : "Show more · ") + shown + " of " + selected.length + " shown";
  };
  more.onclick = appendPage;
  appendPage();
}

function comparisonRow({ items, identity, status, differences }, sides) {
  const item = (items[0][0] || items[1][0]).item;
  const title = item.source_id || (item.document_identity ? "Document " + item.document_identity.id :
    transitionTitle(identity, item));
  const card = node("article", undefined, "panel comparison-row");
  card.dataset.status = status;
  const header = node("div", undefined, "comparison-row-header");
  header.append(node("h3", title), node("span", comparisonStatuses[status], "badge"));
  if (differences.length) header.append(node("small", "Differences · " + differences.join(", "), "muted"));
  if (items.every((entries) => entries.length === 1)) {
    const setups = items.map(([entry]) => new Set(entry.replay.effects
      .filter(({ effect }) => effect.difference_pp !== null).map(({ group }) => group)));
    if (setups.every((side) => side.size)) {
      const shared = [...setups[0]].filter((group) => setups[1].has(group)).length;
      header.append(node("small", shared ? countLabel(shared, "shared replay setup") : "Measurements use different replay setups", "muted"));
    }
  }
  card.append(header);
  const grid = node("div", undefined, "compare-grid");
  for (const [side, entries] of items.entries()) {
    const panel = node("div", undefined, "comparison-side");
    panel.append(node("h4", compareSides[side].toUpperCase()));
    if (!entries.length) panel.append(node("p", "No corresponding capture", "muted"));
    for (const { item, replay, ruleText } of entries) {
      const entry = node("div", undefined, "comparison-entry");
      const { location, position } = sides[side];
      entry.append(contextDetails(item, false));
      const qualifiers = [item.role, item.kind.replaceAll("_", " ")].filter(Boolean);
      entry.append(node("small", qualifiers.join(" · "), "muted"));
      if (ruleText !== undefined) {
        const rule = node("details");
        rule.append(node("summary", "Declared rule"), node("pre", ruleText));
        entry.append(rule);
      }
      const links = contextPatternsByCall.get(JSON.stringify([location.run_id, location.call_id])) || [];
      const evidenceTypes = new Set(links.filter(({ item: link }) =>
        link.context_id === item.id && link.decision_id === location.decision_id
      ).map(({ group }) => contextRelationshipName(group.relationship)));
      if (evidenceTypes.size) entry.append(node("small", [...evidenceTypes].join(" · "), "anchor"));
      if (replay.effects.length) appendContextReplayResults(entry, replay);
      else entry.append(node("small", "No replay report", "muted"));
      const open = node("button", "Open context on " + compareSides[side].toUpperCase(), "occurrence");
      open.addEventListener("click", () => openDecision(position.run, position.call, position.decision, { context_item_id: item.id }));
      entry.append(open);
      panel.append(entry);
    }
    grid.append(panel);
  }
  card.append(grid);
  return card;
}
