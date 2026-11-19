"use strict";

const transitionLabels = {
  matching: "Matching recorded context",
  changed: "Changed",
  capture_changed: "Capture status changed",
  incomplete: "Evidence incomplete",
  first: "Recorded only before",
  second: "Recorded only after",
  ambiguous: "Multiple possible matches",
};
const differenceLabels = {
  version: "Version", authority: "Authority", kind: "Kind", role: "Role",
  content_reference: "Content reference", document_id: "Document ID",
  request_location: "Declared request location",
  capture_status: "Capture status", rule: "Rule", text: "Text",
};
const transitionIssues = {
  ambiguous_order: "Input order across these traces is unavailable.",
  missing_context: "A call in this pair has no recorded context.",
};
const contextCallIndex = new Map();
const transitionsByCall = new Map();
let contextTimelines;
let contextTransitions;
let changesBySource;

function contextCallKey({ run_id, call_id }) {
  return JSON.stringify([run_id, call_id]);
}

function resolveContextAlignment(alignment, positions) {
  const items = alignment.context_ids.map((ids, side) =>
    ids.map((id) => positions[side].contextItems.get(id))
  );
  if (items.some((side) => side.includes(undefined))) {
    throw new Error("The compared inputs changed. Refresh traces to view this comparison.");
  }
  return { ...alignment, items, differences: alignment.differences.map((field) => differenceLabels[field]) };
}

function indexContextHistory() {
  const runIndices = new Map();
  for (const [runIndex, run] of data.runs.entries()) {
    runIndices.set(run.id, runIndex);
    for (const [index, call] of run.calls.entries()) {
      const key = contextCallKey({ run_id: run.id, call_id: call.id });
      const contextItems = new Map(call.context.map((item) => [item.id, item]));
      contextCallIndex.set(key, { run, call, runIndex, index, contextItems });
    }
  }
  contextTimelines = data.context_timelines.map((timeline) => ({
    ...timeline,
    runs: timeline.runs.map((id) => runIndices.get(id)),
    calls: timeline.calls.map((location) => contextCallIndex.get(contextCallKey(location))),
  })).sort((left, right) => left.runs[0] - right.runs[0]);

  contextTransitions = data.context_transitions.map((record, index) => {
    const before = contextCallIndex.get(contextCallKey(record.before));
    const after = contextCallIndex.get(contextCallKey(record.after));
    const changes = (record.changes || []).map((change) => resolveContextAlignment(change, [before, after]));
    const transition = { ...record, index, before, after, changes, issue: transitionIssues[record.state] };
    transitionsByCall.set(contextCallKey(record.after), transition);
    return transition;
  });
  const observations = data.context_source_observations.map(({ earlier_states }) => ({
    earlier_states,
    all: earlier_states.flatMap((state) => state.pairs),
  }));
  changesBySource = data.context_change_patterns.map((group) => {
    const coverage = observations[group.source_observation_index];
    const earlier = coverage.earlier_states[group.earlier_state_index];
    return {
      ...group,
      title: transitionTitle(group.identity),
      comparisonPairs: earlier?.pairs ?? coverage.all,
      occurrences: group.occurrences.map((item) => {
        const pair = contextTransitions[item.transition_index];
        return { ...item, pair, change: pair.changes[item.change_index] };
      }),
    };
  });
}

function transitionTitle(identity, item) {
  switch (identity?.kind) {
    case "source": return identity.id;
    case "tool": return identity.name;
    case "document": return "Document " + identity.id;
    case "tool_result": return "Tool result " + identity.invocation_id;
    default: return [item.role, item.kind.replaceAll("_", " ")].filter(Boolean).join(" · ");
  }
}

function transitionButtons(pair) {
  const actions = node("div", undefined, "transition-actions");
  for (const [label, { runIndex, index }] of [["Open previous call", pair.before], ["Open current call", pair.after]]) {
    const button = node("button", label);
    button.addEventListener("click", () => openDecision(runIndex, index));
    actions.append(button);
  }
  if (pair.before.call.decisions.length === 1 && pair.after.call.decisions.length === 1) {
    const compare = node("button", "Compare recorded decisions");
    compare.addEventListener("click", () => compareTransition(pair));
    actions.append(compare);
  }
  return actions;
}

function transitionRow(pair, change) {
  const row = node("article", undefined, "context-transition");
  const item = change.items[0][0] || change.items[1][0];
  row.append(node("h3", transitionTitle(change.identity, item)), node("span", transitionLabels[change.status], "badge"));
  if (change.differences.length) row.append(node("small", change.differences.join(" · "), "anchor"));
  const sides = node("div", undefined, "compare-grid");
  for (const [index, position] of [pair.before, pair.after].entries()) {
    const side = node("section", undefined, "transition-side");
    side.append(node("h4", index ? "Current call" : "Previous call"));
    for (const item of change.items[index]) {
      const details = contextDetails(item, false);
      const capture = captureOnCall(position.run.id, position.call.id)?.entries.get(item.id)?.capture;
      if (capture) details.append(node("small", captureLabels[capture], "anchor"));
      const open = node("button", "Open recorded context");
      open.addEventListener("click", () => openDecision(position.runIndex, position.index, 0, { context_item_id: item.id }));
      details.append(open);
      side.append(details);
    }
    if (!change.items[index].length) side.append(node("p", "No corresponding entry recorded", "muted"));
    sides.append(side);
  }
  row.append(sides);
  return row;
}

function transitionScope(groups, thread) {
  const pairs = (groups[0]?.occurrences || []).filter((item) =>
    !thread || patternThreadKey(item) === thread
  );
  return { pairs, indices: new Set(pairs.map(({ index }) => index)) };
}

function renderTransitionPatterns(list, primaryScope, scopes, query) {
  const patterns = [{ occurrences: contextTransitions }];
  const primary = scopedPatternGroups(patterns, primaryScope.organization, primaryScope.user);
  updatePatternThreadFilter(primary);
  const comparison = comparisonPatternFilter(scopes, patterns);
  const cohorts = [transitionScope(primary, state.patternThread)];
  if (comparison) cohorts.push(transitionScope(comparison.groups, comparison.thread));
  list.append(transitionCoverage(cohorts, primaryScope, comparison?.scope));

  const groups = changesBySource.flatMap((group) => {
    const description = [group.title, group.adapter, transitionLabels[group.status], ...group.versions.flat(),
      ...group.differences.map((field) => differenceLabels[field])];
    if (!description.filter(Boolean).join(" ").toLowerCase().includes(query)) return [];
    const occurrences = cohorts.map(({ indices }) =>
      group.occurrences.filter((item) => indices.has(item.transition_index)).sort(compareTimes)
    );
    const count = new Set(occurrences.flat().map(({ transition_index }) => transition_index)).size;
    return count ? [{ group, occurrences, count }] : [];
  });
  for (const { group, occurrences } of groups.sort((a, b) => b.count - a.count)) {
    const compared = cohorts.map(({ indices }) =>
      group.comparisonPairs.filter(({ transition_index }) => indices.has(transition_index))
    );
    list.append(transitionPatternCard(group, occurrences, compared));
  }
  if (!groups.length) list.append(node("p", "No named source or tool changes match this scope and filter.", "muted"));
}

function transitionCoverage(cohorts, primaryScope, comparisonScope) {
  if (comparisonScope) {
    const shared = cohorts[1].pairs.filter(({ index }) => cohorts[0].indices.has(index)).length;
    return patternScopeSummary("Input call pairs by scope", [primaryScope, comparisonScope],
      cohorts.map(({ pairs }) => transitionPairCounts(pairs)), countLabel(shared, "input call pair"), "transition-coverage");
  }
  const panel = node("section", undefined, "panel pattern transition-coverage");
  panel.append(node("h2", "Input call pairs in this scope"), transitionPairCounts(cohorts[0].pairs));
  return panel;
}

function transitionPairCounts(pairs) {
  const counts = { compared: 0, missing_context: 0, ambiguous_order: 0 };
  for (const pair of pairs) counts[pair.state] += 1;
  return node("p", [
    countLabel(counts.compared, "pair") + " compared",
    countLabel(counts.missing_context, "pair") + " with missing context",
    countLabel(counts.ambiguous_order, "pair") + " with uncertain order",
  ].join(" · "), "count");
}

function transitionPatternCard(group, occurrences, compared) {
  const card = node("article", undefined, "panel pattern transition-pattern");
  card.append(node("h2", group.title), node("span", transitionLabels[group.status], "badge"));
  if (group.adapter) card.append(node("p", group.adapter, "muted"));
  const sides = group.captures.map((states, index) => {
    if (!states.length) return "No entry recorded";
    return [...group.versions[index], ...states.map((state) => captureLabels[state])].join(" · ");
  });
  card.append(node("p", sides.join(" → "), "muted"));
  if (group.differences.length) card.append(node("small", group.differences.map((field) => differenceLabels[field]).join(" · "), "anchor"));
  if (occurrences.length === 1) {
    appendTransitionPatternEvidence(card, group, occurrences[0], compared[0]);
    return card;
  }
  const columns = node("div", undefined, "compare-grid pattern-scope-columns");
  for (const [index, observed] of occurrences.entries()) {
    const column = patternScopeColumn(index);
    appendTransitionPatternEvidence(column, group, observed, compared[index]);
    columns.append(column);
  }
  card.append(columns);
  return card;
}

function appendTransitionPatternEvidence(container, group, occurrences, compared) {
  if (!compared.length) {
    container.append(node("p", "No qualifying pairs", "transition-frequency"));
    return;
  }
  const threads = new Set(occurrences.map((item) => patternThreadKey(item)).filter(Boolean));
  if (occurrences.length) {
    const counts = [countLabel(occurrences.length, "call pair")];
    if (threads.size) counts.push(countLabel(threads.size, "known thread"));
    container.append(node("p", counts.join(" · "), "count"));
  }
  const basis = group.earlier_state_index === undefined
    ? " with a recorded entry for this source"
    : " with the same earlier versions and capture states";
  container.append(node("p", occurrences.length + " of " + countLabel(compared.length, "pair") + basis, "transition-frequency"));
  if (occurrences.length) {
    const details = node("details");
    details.append(node("summary", "Open call pairs"));
    details.addEventListener("toggle", () => {
      if (!details.open || details.childElementCount > 1) return;
      for (const { pair, change } of occurrences) {
        details.append(transitionOccurrence(pair, change));
      }
    });
    container.append(details);
  }
  if (compared.length > occurrences.length) container.append(comparedSourcePairs(group, compared, occurrences));
  container.append(patternDecisionOutcomes(group, compared, occurrences));
}

function transitionOccurrence(pair, change, status = "") {
  const entry = node("details", undefined, "transition-occurrence");
  const run = pair.after.run;
  const label = [run.organization_id, run.user_id, run.conversation_id || run.id].filter(Boolean).join(" / ");
  const callLabel = (side) => pair.before.run.id === pair.after.run.id ? side.call.id : side.run.id + " / " + side.call.id;
  entry.append(node("summary", [label, callLabel(pair.before) + " → " + callLabel(pair.after), status].filter(Boolean).join(" · ")));
  entry.addEventListener("toggle", () => {
    if (!entry.open || entry.childElementCount > 1) return;
    entry.append(transitionButtons(pair));
    appendDecisionEvidence(entry, pair);
    entry.append(transitionRow(pair, change));
  });
  return entry;
}

function comparedSourcePairs(group, compared, occurrences) {
  const details = node("details", undefined, "transition-compared-pairs");
  details.append(node("summary", "Inspect compared pairs"));
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    appendComparedPairRows(details, group, compared, occurrences);
  });
  return details;
}

function appendComparedPairRows(container, group, compared, occurrences) {
  const changed = new Map(occurrences.map(({ transition_index, change }) => [transition_index, change]));
  const ordered = [...compared].sort((a, b) =>
    compareTimes(contextTransitions[a.transition_index], contextTransitions[b.transition_index])
  );
  const rows = node("div");
  const more = node("button", "Show more pairs");
  const appendPage = () => {
    for (const item of ordered.slice(rows.children.length, rows.children.length + 25)) {
      const pair = contextTransitions[item.transition_index];
      const observed = changed.get(item.transition_index);
      const change = observed ?? pair.changes.find(({ identity }) =>
        identity?.kind === group.identity.kind && transitionTitle(identity) === group.title
      ) ?? resolveContextAlignment({ ...item, identity: group.identity, differences: [] }, [pair.before, pair.after]);
      rows.append(transitionOccurrence(pair, change, observed ? "Pattern observed" : transitionLabels[change.status]));
    }
    more.hidden = rows.children.length === ordered.length;
  };
  more.addEventListener("click", appendPage);
  appendPage();
  container.append(rows, more);
}

function appendTimelineChanges(row, pair) {
  if (!pair) return;
  if (pair.decision_change?.status !== "same") appendDecisionEvidence(row, pair);
  if (pair.issue) {
    row.append(node("p", pair.issue, "muted"));
    return;
  }
  if (!pair.changes.length) return;
  const details = node("details", undefined, "timeline-changes timeline-context-changes");
  details.append(node("summary", countLabel(pair.changes.length, "context difference") + " from previous input"));
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    details.append(transitionButtons(pair));
    if (pair.decision_change?.status === "same") appendDecisionEvidence(details, pair);
    for (const change of pair.changes) details.append(transitionRow(pair, change));
  });
  row.append(details);
}
