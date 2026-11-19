"use strict";

const decisionChangeLabels = {
  mix_changed: "Recorded decision mix changed",
  outcome_changed: "Recorded outcomes changed",
  same: "Same recorded decisions",
  incomplete: "Outcome evidence incomplete",
  unavailable: "Decision evidence unavailable",
};

function appendDecisionEvidence(container, pair) {
  const change = pair.decision_change;
  if (!change) return;
  const details = node("details", undefined, "transition-decision-evidence");
  details.append(node("summary", decisionChangeLabels[change.status]));
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    for (const group of change.groups) {
      details.append(decisionGroupEvidence(pair, group, change.groups.length > 1));
    }
    if (!change.groups.length) details.append(node("p", "Neither call has a recorded decision.", "muted"));
  });
  container.append(details);
}

function decisionGroupEvidence(pair, group, showStatus) {
  const row = node("article", undefined, "transition-decision-group");
  row.append(node("h4", [labels[group.kind] || group.kind, group.label].filter(Boolean).join(" · ")));
  if (showStatus) row.append(node("span", decisionChangeLabels[group.status], "badge"));
  const sides = node("div", undefined, "compare-grid");
  const positions = [pair.before, pair.after];
  const indices = group.decision_ids.map((ids, index) =>
    ids.map((id) => positions[index].call.decisions.findIndex((decision) => decision.id === id))
  );
  for (const [side, position] of positions.entries()) {
    const column = node("div", undefined, "decision-evidence-side");
    const heading = side ? "Current call" : "Previous call";
    column.append(node("p", heading + " · " + countLabel(indices[side].length, "recorded decision"), "muted"));
    for (const index of indices[side]) {
      const decision = position.call.decisions[index];
      const label = indices[side].length === 1 ? "Open recorded decision" : "Open decision " + decision.id;
      const button = node("button", label);
      button.addEventListener("click", () => openDecision(position.runIndex, position.index, index));
      column.append(button);
      if (decision.outcome === null) column.append(node("small", "Outcome unavailable", "anchor"));
    }
    sides.append(column);
  }
  row.append(sides);
  if (indices.every((side) => side.length === 1)) {
    const compare = node("button", "Compare these decisions");
    compare.addEventListener("click", () => compareTransition(pair, indices.map(([index]) => index)));
    row.append(compare);
  }
  return row;
}

function patternDecisionOutcomes(group, compared, occurrences) {
  const details = node("details", undefined, "transition-decision-outcomes");
  details.append(node("summary", "Decision outcomes across compared pairs"));
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    const observed = new Set(occurrences.map(({ transition_index }) => transition_index));
    const outcomes = new Map(Object.keys(decisionChangeLabels).map((status) => [status, [[], []]]));
    for (const pair of compared) {
      const status = contextTransitions[pair.transition_index].decision_change?.status ?? "unavailable";
      outcomes.get(status)[observed.has(pair.transition_index) ? 0 : 1].push(pair);
    }
    const cohorts = ["Pattern observed", "Other compared pairs"];
    const table = node("table", undefined, "effects-table transition-decision-table");
    const heading = table.createTHead().insertRow();
    for (const label of ["Recorded decisions", ...cohorts]) {
      const cell = node("th", label);
      cell.scope = "col";
      heading.append(cell);
    }
    const drill = node("section", undefined, "transition-decision-drill");
    const body = table.createTBody();
    for (const [status, bins] of outcomes) {
      if (!bins.some((bin) => bin.length)) continue;
      const row = body.insertRow();
      const caption = node("th", decisionChangeLabels[status]);
      caption.scope = "row";
      row.append(caption);
      for (const [index, pairs] of bins.entries()) {
        const cell = row.insertCell();
        if (!pairs.length) {
          cell.textContent = "0";
          continue;
        }
        const button = node("button", String(pairs.length));
        const title = cohorts[index] + " · " + decisionChangeLabels[status];
        button.dataset.decisionStatus = status;
        button.dataset.cohort = index ? "other" : "observed";
        button.setAttribute("aria-label", title + " · " + countLabel(pairs.length, "pair"));
        button.setAttribute("aria-pressed", "false");
        button.addEventListener("click", () => {
          for (const control of table.querySelectorAll("button")) {
            control.setAttribute("aria-pressed", String(control === button));
          }
          drill.replaceChildren(node("h3", title));
          appendComparedPairRows(drill, group, pairs, occurrences);
        });
        cell.append(button);
      }
    }
    const scroll = node("div", undefined, "effects-scroll");
    scroll.append(table);
    details.append(scroll, drill);
  });
  return details;
}
