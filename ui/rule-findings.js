"use strict";

const findingsByCall = new Map();
const recurringPatternsByFinding = new Map();
let findingOccurrences = [];

function indexFindings() {
  if (!data.findings.length) return;
  const runs = new Map(data.runs.map((run) => [run.id, run]));
  findingOccurrences = data.findings.map((finding, finding_index) => {
    const run = runs.get(finding.run_id);
    const call = run.calls.find((item) => item.id === finding.call_id);
    const decision = call.decisions.find((item) => item.id === finding.decision_id);
    const occurrence = {
      ...finding, finding_index, decision_label: decision.label,
      organization_id: run.organization_id, user_id: run.user_id, conversation_id: run.conversation_id,
      trace_source: run.source, started_at: call.started_at, turn_id: call.turn_id,
    };
    const key = JSON.stringify([finding.run_id, finding.call_id]);
    if (!findingsByCall.has(key)) findingsByCall.set(key, []);
    findingsByCall.get(key).push(occurrence);
    return occurrence;
  });
  for (const [index, pattern] of data.patterns.entries()) {
    if (pattern.organization_id !== null || pattern.user_id !== null) continue;
    for (const findingIndex of pattern.occurrences) recurringPatternsByFinding.set(findingIndex, index);
  }
}

function callFindings(runId, callId, decisionId) {
  const findings = findingsByCall.get(JSON.stringify([runId, callId])) || [];
  return decisionId === undefined ? findings : findings.filter((item) => item.decision_id === decisionId);
}

function findingReplayTests(finding) {
  const individual = finding.context_ids.map((context_id) =>
    contextReplay({ ...finding, context_id }).effects.filter(({ group }) => !group.pair)
  );
  // An exact joint test is indexed under both inputs, so read it from the first input only.
  const joint = contextReplay({ ...finding, context_id: finding.context_ids[0] }).effects.filter(({ effect }) =>
    effect.context_ids?.length === 2 && finding.context_ids.every((id) => effect.context_ids.includes(id))
  );
  return { individual, joint };
}

function hasReplayEffect(effects) {
  return effects.some(({ effect }) => effect.difference_pp !== null);
}

function renderFindingCards(container, run, call, decision, canReplay) {
  const matches = decision ? callFindings(run.id, call.id, decision.id) : [];
  for (const finding of matches) {
    const card = node("article", undefined, "finding " + finding.kind);
    card.dataset.findingIndex = finding.finding_index;
    card.append(node("span", labels[finding.kind], "badge"));
    card.append(node("h3", finding.key + " · " + finding.scope));
    card.append(node("p", "Derived from annotated rules in this call.", "muted"));
    const { individual, joint } = findingReplayTests(finding);
    for (const [index, id] of finding.context_ids.entries()) {
      const details = contextDetails(call.context.find((item) => item.id === id), true);
      const effects = individual[index];
      appendContextReplayResults(details, { effects, status: hasReplayEffect(effects) ? "measured" : "unavailable" });
      card.append(details);
    }
    appendContextReplayResults(card, { effects: joint }, hasReplayEffect(joint) ? "Joint effect measured" : "Joint effect unavailable");
    if (canReplay && finding.kind === "conflict") {
      const prepare = node("button", "Test this conflict");
      prepare.addEventListener("click", () => openReplayPlanner(run, call, decision, finding.context_ids));
      card.append(prepare);
    }
    const patternIndex = recurringPatternsByFinding.get(finding.finding_index);
    if (patternIndex !== undefined) {
      const pattern = node("button", "View finding across threads");
      pattern.addEventListener("click", () => openRulePattern(patternIndex, finding.key));
      card.append(pattern);
    }
    card.append(evidenceLinkButton({ finding_index: finding.finding_index }, "this finding"));
    container.append(card);
  }
  if (!matches.length) {
    const message = decision?.applies_to ? "No conflicting or duplicate rules found in this scope." :
      "Conflict analysis requires annotated rules and a decision scope.";
    container.append(node("p", message, "muted"));
  }
  return matches.length > 0;
}

function findingLink(finding, label) {
  return occurrenceButton(finding, label, { finding_index: finding.finding_index });
}

function findingCounts(findings) {
  const conflicts = findings.filter(({ kind }) => kind === "conflict").length;
  const duplicates = findings.filter(({ kind }) => kind === "duplicate").length;
  const incomplete = findings.length - conflicts - duplicates;
  return [
    conflicts && countLabel(conflicts, "conflict"),
    duplicates && countLabel(duplicates, "duplicate finding"),
    incomplete && countLabel(incomplete, "finding") + " with incomplete evidence",
  ].filter(Boolean);
}

function renderTimelineFindings(row, run, call) {
  const findings = callFindings(run.id, call.id);
  if (!findings.length) return;
  const details = node("details", undefined, "timeline-changes timeline-findings");
  details.append(node("summary", findingCounts(findings).join(" · ")));
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    for (const item of findings) {
      details.append(findingLink(item, [labels[item.kind], item.key, item.scope, item.decision_label].join(" · ")));
    }
  });
  row.append(details);
}

function renderRulePatterns(list, organization, user, query) {
  const groups = data.patterns.flatMap((group, index) =>
    group.organization_id === organization && group.user_id === user
      ? [{ ...group, index, occurrences: group.occurrences.map((index) => findingOccurrences[index]) }] : []
  );
  updatePatternThreadFilter(groups);
  for (const pattern of groups) {
    if (![pattern.key, pattern.scope, labels[pattern.kind], ...pattern.sources.flat()].join(" ").toLowerCase().includes(query)) continue;
    const occurrences = pattern.occurrences.filter((item) =>
      !state.patternThread || patternThreadKey(item) === state.patternThread
    );
    if (!occurrences.length) continue;
    const card = node("article", undefined, "panel pattern rule-pattern " + pattern.kind);
    card.dataset.rulePatternIndex = pattern.index;
    card.append(node("span", labels[pattern.kind], "badge"));
    card.append(node("h2", pattern.key + " · " + pattern.scope));
    card.append(node("p", pattern.sources.map(([id, version]) => id + " " + version).join(" / "), "muted"));
    const runs = state.patternThread ? new Set(occurrences.map((item) => item.run_id)).size : pattern.affected_runs;
    const threads = state.patternThread ? 1 : pattern.affected_conversations;
    const knownTurns = new Set(occurrences.map((item) => item.turn_id));
    const turns = state.patternThread ? (knownTurns.has(null) ? null : knownTurns.size) : pattern.affected_turns;
    const counts = [countLabel(runs, "run")];
    if (threads !== null) counts.push(countLabel(threads, "conversation"));
    if (turns !== null) counts.push(countLabel(turns, "turn"));
    card.append(node("p", counts.join(" · "), "count"));
    const rows = occurrences.sort(compareTimes).map((item) => {
      const { individual, joint } = findingReplayTests(item);
      return {
        item, singleMeasured: individual.some(hasReplayEffect), jointMeasured: hasReplayEffect(joint),
        tested: individual.some((effects) => effects.length) || joint.length > 0,
      };
    });
    const individuals = rows.filter((row) => row.singleMeasured).length;
    const pairs = rows.filter((row) => row.jointMeasured).length;
    if (individuals || pairs) {
      card.append(node("p", individuals + " of " + occurrences.length + " findings with an individual effect measured · "
        + pairs + " with a joint effect measured", "muted"));
    }
    for (const { item, singleMeasured, jointMeasured, tested } of rows) {
      const label = [item.organization_id, item.user_id, item.conversation_id || item.run_id].filter(Boolean).join(" / ");
      const call = [item.decision_label, item.turn_id, item.call_id].filter(Boolean).join(" · ");
      const occurrence = node("div", undefined, "rule-occurrence");
      occurrence.append(findingLink(item, label + " · " + call), conversationOccurrenceButton(item));
      const measured = [singleMeasured && "Individual effect measured", jointMeasured && "Joint effect measured"].filter(Boolean);
      if (tested && !measured.length) measured.push("Replay effect unavailable");
      const provenance = [item.started_at, ...measured].filter(Boolean).join(" · ");
      if (provenance) occurrence.append(node("small", provenance, "anchor"));
      card.append(occurrence);
    }
    list.append(card);
  }
  if (!list.children.length) list.append(node("p", query
    ? "No findings match this scope and filter." : "No findings with identified source versions in this scope.", "muted"));
}

function openRulePattern(index, search) {
  state.view = "patterns";
  state.patternThread = "";
  state.patternScope = defaultPatternScope;
  element("pattern-type").value = "rules";
  element("pattern-search").value = search;
  render();
  const target = element("pattern-list").querySelector(`[data-rule-pattern-index="${index}"]`);
  if (target) {
    target.tabIndex = -1;
    target.focus();
  }
}
