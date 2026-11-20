"use strict";

const element = (id) => document.getElementById(id);
const state = { view: "conversation", run: 0, call: 0, decision: 0, patternThread: "", timelineCall: null };
const patternComparison = { scope: "", thread: "" };
const labels = {
  conflict: "Conflict", duplicate: "Duplicate", insufficient_evidence: "Insufficient evidence",
  included: "Included", redacted: "Redacted", unknown: "Availability unknown",
  tool_call: "Tool call", model_output: "Model answer", tool_execution: "Tool execution",
  decision_model: "Decision model", guardrail: "Guardrail",
};
let data;
const contextPatternsByCall = new Map();
const preflightChecksByCall = new Map();
const candidateProbesByCall = new Map();
const shadowProbesByCall = new Map();
const replaysByContext = new Map();
const recordedRequests = new Set();
const noContextReplay = { status: "untested", effects: [] };
const contextReplayStatus = {
  measured: "Individual effect measured",
  unavailable: "Individual effect unavailable",
  untested: "No replay report",
};
const replayDecisionResults = [
  ["lower", "Lower match rate"],
  ["higher", "Higher match rate"],
  ["unchanged", "No observed change"],
  ["mixed", "Mixed results"],
  ["unavailable", "Effect unavailable"],
];

function node(tag, text, className) {
  const item = document.createElement(tag);
  if (text !== undefined) item.textContent = text;
  if (className) item.className = className;
  return item;
}

function countLabel(count, label) {
  return count + " " + label + (count === 1 ? "" : "s");
}

function options(select, values, current) {
  select.replaceChildren(...values.map((value, index) => new Option(value, index)));
  if (values.length) select.value = current;
  select.disabled = values.length < 2;
}

function evidence(anchor) {
  const pointer = [anchor.span_id, anchor.field];
  if (anchor.message_index !== null) pointer.push("message " + (anchor.message_index + 1));
  if (anchor.part_index !== null) pointer.push("part " + (anchor.part_index + 1));
  return node("small", pointer.filter(Boolean).join(" / "), "anchor");
}

function readable(value) {
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function requestFieldLabel(pointer, range) {
  return pointer + (range ? " · UTF-8 bytes " + range.start_byte + " to " + range.end_byte + " (end excluded)" : "");
}

function contextDetails(item, open) {
  const details = node("details", undefined, "context-item");
  details.open = open;
  const summary = node("summary");
  const title = [item.source_id, item.source_version].filter(Boolean).join(" · ");
  const kind = item.kind === "message" && Number.isInteger(item.evidence.message_index)
    ? "message " + (item.evidence.message_index + 1) : item.kind.replaceAll("_", " ");
  const type = item.document_identity ? "Document " + item.document_identity.id :
    [item.role, kind].filter(Boolean).join(" · ");
  summary.append(node("strong", title || type));
  summary.append(node("span", labels[item.availability], "badge"));
  details.append(summary);
  if (item.authority) details.append(node("small", item.authority, "muted"));
  if (item.text !== null) details.append(node("pre", item.text));
  if (item.content_reference && item.content_reference !== item.source_id) {
    details.append(node("small", item.content_reference, "anchor"));
  }
  if (item.document_identity && item.document_identity.evidence.field !== item.evidence.field) {
    details.append(evidence(item.document_identity.evidence));
  }
  if (item.request_fragment) {
    const { pointer, text_range } = item.request_fragment;
    details.append(node("small", "Declared request fragment " + requestFieldLabel(pointer, text_range), "anchor"));
  }
  details.append(evidence(item.evidence));
  return details;
}

function openDecision(runIndex, callIndex, decisionIndex = 0, reference) {
  if (state.view === "conversation") {
    conversationReturn = { run: state.run, call: state.call, decision: state.decision, scroll: window.scrollY };
  }
  linkNotice();
  state.run = runIndex;
  state.call = callIndex;
  state.decision = decisionIndex;
  state.view = "decisions";
  render();
  const outcome = element("outcome");
  let target;
  if (reference?.context_item_id) {
    target = [...element("context").querySelectorAll(".context-item")]
      .find((item) => item.dataset.contextId === reference.context_item_id);
    if (target) target.open = true;
  } else if (reference?.retrieved_document_id) {
    target = [...outcome.querySelectorAll("[data-retrieved-document-id]")]
      .find((item) => item.dataset.retrievedDocumentId === reference.retrieved_document_id);
    if (target) target.open = true;
  } else if (reference?.finding_index !== undefined) {
    target = element("findings").querySelector(`[data-finding-index="${reference.finding_index}"]`);
  } else if (reference?.capture_profile) {
    target = element("capture-on-call");
  } else if (reference?.evidence) {
    const { kind, index } = reference.evidence;
    target = outcome.querySelector(`[data-context-reference="${kind}:${index}"]`);
  } else if (reference?.activation_id !== undefined) {
    target = openActivationCheckpoint(reference);
  } else if (reference) {
    const replay = [...element("decision-replays").querySelectorAll(".replay-evidence")].find(
      (item) => item.dataset.replayId === reference.replay_id
    );
    if (reference.control_context_id !== undefined) {
      const comparison = replay?.querySelector(".replay-negative-control");
      if (comparison?.dataset.controlContextId === reference.control_context_id) target = comparison;
    } else {
      target = replay && (reference.context_id === undefined
        ? replay.querySelector(".replay-pair")
        : [...replay.querySelectorAll("[data-context-id]")].find((item) => item.dataset.contextId === reference.context_id));
    }
  }
  const found = !reference || Boolean(target);
  state.reference = found ? reference : undefined;
  target ||= outcome;
  target.tabIndex = -1;
  target.focus();
  return found;
}

function renderContextLinks(outcome, run, call, decision, links) {
  if (!links.length) return;
  const section = node("section", undefined, "automatic-links");
  section.append(node("h2", "Context found by exact match"));
  section.append(node("p", "These matches suggest which inputs to test. Replay measures whether changing an input changes the decision.", "muted"));
  for (const { link, index } of links) {
    const item = call.context.find((item) => item.id === link.context_id);
    const details = contextDetails(item, false);
    const title = {
      tool_definition: "Matching tool definition",
      argument_match: "Matching value for argument " + link.argument_path,
    }[link.kind] || contextRelationshipName(link);
    const card = node("article", undefined, "finding");
    card.dataset.contextReference = "exact_match:" + index;
    card.append(node("h3", title));
    card.append(node("blockquote", link.excerpt));
    if (link.context_path) card.append(node("small", "Path in decoded context " + link.context_path, "anchor"));
    card.append(details);
    if (link.source_tool) {
      card.append(occurrenceButton({ run_id: run.id, ...link.source_tool }, "Open source tool call " + link.source_tool.tool));
    }
    appendContextReplayResults(card, contextReplay(link));
    if (decision.outcome !== null && item.availability === "included" && item.text !== null) {
      const prepare = node("button", "Test this context");
      prepare.addEventListener("click", () => openReplayPlanner(run, call, decision, [item.id]));
      card.append(prepare);
    }
    card.append(evidenceLinkButton({ evidence: { kind: "exact_match", index } }, title.toLowerCase()));
    section.append(card);
  }
  outcome.append(section);
}

function renderReplays(container, run, call, decision) {
  const sets = data.replays.filter(({ location }) =>
    location.run_id === run.id && location.call_id === call.id && location.decision_id === decision.id
  );
  for (const experiment of sets) {
    const section = node("section", undefined, "panel replay-evidence");
    section.dataset.replayId = experiment.id;
    section.append(node("h2", "Controlled replay"));
    section.append(node("small", experiment.runner_id + " · " + replayMatchLabel(experiment.match_rule), "anchor"));
    section.append(node("small", replayBaselineLabel(experiment.location), "anchor"));
    if (experiment.sampling_design === "independent") {
      section.append(node("small", "Independent sampling declared", "anchor"));
    }
    if (experiment.rescored_from) section.append(node("small", "Rescored from " + experiment.rescored_from, "anchor"));
    section.append(node("p", replayCount("Baseline", experiment.baseline_summary)));
    const control = experiment.control_comparison;
    if (control) section.append(replayControlCard(experiment, call, control));
    if (experiment.pair) section.append(replayPairCard(experiment, call));
    for (const condition of experiment.variants) {
      const { context_id: contextId, pointer, operation, text_range } = condition.intervention;
      const context = call.context.find((item) => item.id === contextId);
      const card = node("article", undefined, "finding");
      card.dataset.contextId = contextId;
      const role = control ? (contextId === control.control_context_id ? "Negative control · " : "Target · ") : "";
      card.append(node("h3", role + (operation === "remove_json" ? "Removed " : "Replaced ") + (context.source_id || context.id)));
      card.append(node("p", replayCount("Variant", condition.summary)));
      if (condition.difference_pp === null) {
        card.append(node("p", "Effect unavailable. Check baseline matches, failed trials, and unscored outcomes.", "muted"));
      } else {
        const effect = (condition.difference_pp > 0 ? "+" : "") + condition.difference_pp.toFixed(1);
        card.append(node("p", effect + " percentage points in the recorded decision rate", "count"));
      }
      appendReplayInterval(card, condition.difference_95_interval_pp);
      appendReplayOutcomes(card, experiment.location, experiment.id, contextId);
      const details = node("details");
      details.append(node("summary", "Intervention and trial outcomes"));
      details.append(evidence(context.evidence));
      details.append(node("small", "Request path " + requestFieldLabel(pointer, text_range), "anchor"));
      details.append(node("p", "Captured context", "muted"), node("pre", context.text));
      showReplayEdit(details, condition.intervention);
      details.append(node("p", "Variant trial outcomes", "muted"), node("pre", readable(condition.trials)));
      card.append(details, evidenceLinkButton({ replay_id: experiment.id, context_id: contextId }, "this replay variant"));
      section.append(card);
    }
    const baseline = node("details");
    baseline.append(
      node("summary", "Baseline trial outcomes"),
      node("small", "Request SHA-256 " + experiment.baseline_request_sha256, "anchor"),
      node("pre", readable(experiment.baseline))
    );
    section.append(baseline);
    container.append(section);
  }
}

function hasRecordedRequest(runId, callId) {
  return recordedRequests.has(JSON.stringify([runId, callId]));
}

function replayBaselineLabel(location) {
  return hasRecordedRequest(location.run_id, location.call_id)
    ? "Baseline matched the captured request" : "Baseline request provenance unverified";
}

function showReplayEdit(container, change) {
  if (change.operation === "remove_json") {
    container.append(node("p", "Removed this JSON value", "muted"));
  } else {
    const label = {
      replace_json: "Replacement JSON", replace_text_fragment: "Replacement for text fragment",
    }[change.operation] || "Replacement text";
    container.append(node("p", label, "muted"), node("pre", change.replacement || "(empty text)"));
  }
}

function replayControlCard(experiment, call, control) {
  const card = node("article", undefined, "finding replay-negative-control");
  card.dataset.controlContextId = control.control_context_id;
  card.append(node("h3", "Target compared with negative control"));
  const contrast = control.target_minus_control_pp;
  if (contrast === null) {
    card.append(node("p", "Comparison unavailable. Check baseline matches, failed trials, and unscored outcomes.", "muted"));
  } else {
    card.append(node("p", effectText(contrast) + " in the recorded decision rate (target edit minus control edit)", "count"));
    appendReplayInterval(card, control.target_minus_control_95_interval_pp);
  }
  for (const { intervention: { context_id: contextId } } of experiment.variants) {
    const item = call.context.find((entry) => entry.id === contextId);
    const role = contextId === control.control_context_id ? "Negative control" : "Target";
    const label = node("p", role + " · ");
    const link = node("a", [item.source_id || item.id, item.source_version].filter(Boolean).join(" "));
    link.href = inspectionLink({ replay_id: experiment.id, context_id: contextId });
    label.append(link);
    card.append(label);
  }
  card.append(evidenceLinkButton({
    replay_id: experiment.id, control_context_id: control.control_context_id,
  }, "this target and control comparison"));
  return card;
}

function replayPairCard(experiment, call) {
  const { pair, variants: [left, right], baseline_summary: baseline } = experiment;
  const names = [left, right].map((variant) => {
    const item = call.context.find((context) => context.id === variant.intervention.context_id);
    return [item.source_id || item.id, item.source_version].filter(Boolean).join(" ");
  });
  const card = node("article", undefined, "finding replay-pair");
  card.append(node("h3", "Two context changes together"));
  const table = node("table", undefined, "effects-table");
  table.append(node("caption", "Trials that matched the recorded decision"));
  const heading = table.createTHead().insertRow();
  for (const title of [names[0], names[1] + " original", names[1] + " replaced"]) {
    const cell = node("th", title);
    cell.scope = "col";
    heading.append(cell);
  }
  const rows = table.createTBody();
  for (const [label, summaries] of [["Original", [baseline, right.summary]], ["Replaced", [left.summary, pair.summary]]]) {
    const row = rows.insertRow();
    const cell = node("th", label);
    cell.scope = "row";
    row.append(cell);
    for (const summary of summaries) row.insertCell().textContent = trialCounts(summary);
  }
  const scroll = node("div", undefined, "effects-scroll");
  scroll.append(table);
  card.append(scroll);
  const joint = pair.difference_pp === null ? "Unavailable" : effectText(pair.difference_pp);
  const interaction = pair.interaction_pp === null ? "Unavailable" : effectText(pair.interaction_pp);
  card.append(node("p", "Joint change · " + joint + "   |   Interaction · " + interaction, "count"));
  appendReplayInterval(card, pair.difference_95_interval_pp, "Joint change");
  appendReplayInterval(card, pair.interaction_95_interval_pp, "Interaction");
  card.append(node("p", "Interaction is the joint change minus the sum of the two individual changes.", "muted"));
  appendReplayOutcomes(card, experiment.location, experiment.id);
  const details = node("details");
  details.append(node("summary", "Trials with both texts replaced"), node("pre", readable(pair.trials)));
  card.append(details, evidenceLinkButton({ replay_id: experiment.id }, "this joint replay"));
  return card;
}

function replayCount(label, summary) {
  const scored = summary.completed - (summary.unscored || 0);
  const trials = summary.unscored ? " scored trials" : " completed trials";
  return label + " matched the recorded decision in " + summary.matched + " of " + scored + trials + trialGaps(summary);
}

function renderDecision() {
  const run = data.runs[state.run];
  const call = run.calls[state.call];
  const decision = call.decisions[state.decision];
  const capture = captureOnCall(run.id, call.id);
  const canReplay = decision && decision.kind !== "tool_execution" && decision.outcome !== null
    && call.context.some((item) => item.availability === "included" && item.text !== null);
  options(element("run"), data.runs.map((item) =>
    item.conversation_id && item.source.startsWith("langsmith")
      ? item.conversation_id + " / " + item.id.replace(/^langsmith:/, "")
      : item.conversation_id || item.id
  ), state.run);
  options(element("call"), run.calls.map((item, index) =>
    (index + 1) + " · " + (item.retrieval?.label || item.decisions.map((decision) => decision.label).join(", ") || item.model || item.id)
  ), state.call);
  options(element("decision"), call.decisions.map((item) => item.label), state.decision);
  element("decision-conversation").textContent = conversationReturn ? "Back to conversation" : "Open conversation";
  const outcome = element("outcome");
  const replays = element("decision-replays");
  const activations = element("decision-activations");
  outcome.replaceChildren();
  replays.replaceChildren();
  activations.replaceChildren();
  activations.hidden = true;
  if (call.retrieval && !decision) {
    renderRetrieval(outcome, run, call);
  } else if (!decision) {
    outcome.append(node("h1", "Outcome unavailable"));
  } else {
    outcome.append(node("div", labels[decision.kind] || "Observed decision", "eyebrow"));
    outcome.append(node("h1", decision.label));
    if (decision.outcome === null) {
      outcome.append(node("p", "Outcome body unavailable", "muted"));
    } else {
      outcome.append(node("pre", readable(decision.outcome)));
    }
    outcome.append(evidence(decision.evidence));
    const exactLinks = exactDecisionLinks(run, call, decision);
    renderDecisionInvestigation(outcome, run, call, decision, capture, exactLinks);
    const link = data.tool_links.find((item) =>
      item.run_id === run.id && item.call_id === call.id && item.decision_id === decision.id
    );
    if (link) {
      outcome.append(node("h2", "Recorded tool result"));
      outcome.append(node("pre", link.result.outcome === null ? "Result body unavailable" : readable(link.result.outcome)));
      outcome.append(evidence(link.result.evidence));
      const button = node("button", "Open call with tool result");
      button.addEventListener("click", () => {
        openDecision(state.run, run.calls.findIndex((item) => item.id === link.result_call_id));
      });
      outcome.append(button);
    } else if (decision.tool_call_id) {
      outcome.append(node("p", "Matching tool result unavailable", "muted"));
    }
    for (const [index, link] of call.relationships.entries()) {
      if (link.decision_id !== decision.id) continue;
      const text = link.relation + " · " + link.basis;
      const details = contextDetails(call.context.find((item) => item.id === link.context_id), false);
      const finding = node("article", undefined, "finding");
      finding.dataset.contextReference = "annotation:" + index;
      finding.append(node("h3", text));
      if (link.excerpt) finding.append(node("blockquote", link.excerpt));
      if (link.evidence) details.append(evidence(link.evidence));
      finding.append(details);
      appendContextReplayResults(finding, contextReplay({ run_id: run.id, call_id: call.id, ...link }));
      if (canReplay && details.querySelector("pre")) {
        const prepare = node("button", "Test this context");
        prepare.addEventListener("click", () => openReplayPlanner(run, call, decision, [link.context_id]));
        finding.append(prepare);
      }
      finding.append(evidenceLinkButton({ evidence: { kind: "annotation", index } }, "this context relationship"));
      outcome.append(finding);
    }
    renderContextLinks(outcome, run, call, decision, exactLinks);
    const actions = node("div", undefined, "decision-actions");
    const reports = activationReports(run, call, decision);
    if (reports.length) {
      activations.hidden = false;
      const heading = node("h2", "Activation probe");
      heading.tabIndex = -1;
      activations.append(heading);
      renderActivationReports(activations, reports, call);
      const button = node("button", "View activation probe");
      button.addEventListener("click", () => heading.focus());
      actions.append(button);
    }
    renderReplays(replays, run, call, decision);
    if (replays.firstElementChild) {
      const results = node("button", "View replay results");
      results.addEventListener("click", () => {
        const heading = replays.querySelector("h2");
        heading.tabIndex = -1;
        heading.focus();
      });
      actions.append(results);
    }
    if (canReplay) {
      const prepare = node("button", "Plan replay");
      prepare.addEventListener("click", () => openReplayPlanner(run, call, decision));
      actions.append(prepare);
    }
    const compare = node("button", "Compare decision");
    compare.addEventListener("click", openComparison);
    actions.append(compare);
    outcome.append(actions);
  }
  if (run.spans?.length) {
    const tree = node("button", "Show in execution tree");
    tree.addEventListener("click", () => openExecutionTree(state.run, state.call));
    outcome.append(tree);
  }

  const context = element("context");
  context.replaceChildren(...call.context.map((item) => {
    const details = contextDetails(item, false);
    details.dataset.contextId = item.id;
    const itemCapture = capture?.entries.get(item.id);
    renderItemCapture(details, itemCapture, item.request_fragment);
    if (item.document_identity) renderDocumentHandoff(details, run, call, item);
    if (item.source_id?.trim()) {
      const history = node("button", "View source history");
      history.addEventListener("click", () => openSourceHistory(item.source_id));
      details.append(history);
    }
    if (canReplay && itemCapture?.matching_fields > 0) {
      const prepare = node("button", "Test this context");
      prepare.addEventListener("click", () => openReplayPlanner(run, call, decision, [item.id]));
      details.append(prepare);
    }
    details.append(evidenceLinkButton({ context_item_id: item.id }, "this context entry"));
    return details;
  }));
  if (!call.context.length) context.append(node("p", "Captured context unavailable", "muted"));
  renderRecordedPreflight(context, run, call, false);
  renderRecordedCandidateProbes(context, run, call, false);
  renderRecordedShadowProbes(context, run, call, false);
  renderContextScreen(context, run, call, false, decision);
  if (call.context_metadata) {
    const { issue, evidence: anchor } = call.context_metadata;
    const declaration = node("div", undefined, issue ? "finding insufficient_evidence" : "context-declaration");
    declaration.append(node("small", issue ? "Context declarations skipped" : "Context declarations", issue ? "badge" : "muted"));
    if (issue) declaration.append(node("p", issue));
    declaration.append(evidence(anchor));
    context.prepend(declaration);
  }
  renderPromptSourceLayout(context, call, capture);
  renderCaptureSummary(context, capture, true);

  const findings = element("findings");
  findings.replaceChildren();
  const hasFindings = renderFindingCards(findings, run, call, decision, canReplay);
  const findingsPanel = findings.parentElement;
  const evidenceColumn = findingsPanel.parentElement;
  if (hasFindings) {
    evidenceColumn.prepend(findingsPanel);
  } else {
    evidenceColumn.append(findingsPanel);
  }
}

function renderTimeline() {
  const threads = timelineThreads();
  const active = threads.findIndex((thread) => thread.runs.includes(state.run));
  const thread = threads[active];
  options(element("timeline-run"), threads.map((item) => item.label), active);
  const list = element("timeline-list");
  const tree = element("timeline-tree");
  const order = element("timeline-order");
  const hasTopology = thread.runs.some((runIndex) => data.runs[runIndex].spans?.length);
  element("timeline-layout-control").hidden = !hasTopology;
  const showTree = hasTopology && element("timeline-layout").value === "execution";
  tree.hidden = !showTree;
  list.hidden = showTree;
  if (showTree) {
    order.hidden = true;
    renderExecutionTree(tree, thread);
    return;
  }
  list.replaceChildren();
  const { calls } = thread;
  order.hidden = thread.has_timestamps;
  order.textContent = "Some timestamps are unavailable. Steps without timestamps follow the order in the imported data.";
  const replaysByCall = new Map();
  for (const report of data.replays) {
    const key = JSON.stringify([report.location.run_id, report.location.call_id]);
    if (!replaysByCall.has(key)) replaysByCall.set(key, []);
    replaysByCall.get(key).push(report);
  }
  for (const [position, { call, index, runIndex }] of calls.entries()) {
    const run = data.runs[runIndex];
    const row = node("li");
    row.dataset.timelineCall = runIndex + ":" + index;
    const label = "Step " + (position + 1) + (call.turn_id ? " · " + call.turn_id : "");
    row.append(node("h2", label));
    if (call.started_at) row.append(node("p", call.started_at, "muted"));
    if (thread.runs.length > 1) row.append(node("p", run.id, "anchor"));
    const capture = captureOnCall(run.id, call.id);
    const decisions = countLabel(call.decisions.length, "recorded decision");
    row.append(node("p", capture ? decisions : countLabel(call.context.length, "context item") + " · " + decisions, "muted"));
    renderCaptureSummary(row, capture);
    renderRecordedPreflight(row, run, call, false);
    renderRecordedCandidateProbes(row, run, call, false);
    renderRecordedShadowProbes(row, run, call, false);
    renderContextScreen(row, run, call, false);
    renderTimelineFindings(row, run, call);
    const replays = replaysByCall.get(JSON.stringify([run.id, call.id]));
    if (replays) {
      const variants = replays.flatMap((report) => [...report.variants, ...(report.pair ? [report.pair] : [])]
        .map((variant) => ({ report, variant })));
      const effects = variants.filter(({ variant }) => variant.difference_pp !== null);
      const { report, variant } = effects.find(({ variant }) => !variant.intervention) || effects[0] || variants[0];
      const indexOfDecision = call.decisions.findIndex((decision) => decision.id === report.location.decision_id);
      const measured = node("button", effects.length + " of " + variants.length + " replay effects measured", "replay-timeline");
      measured.addEventListener("click", () => openDecision(runIndex, index, indexOfDecision, {
        replay_id: report.id, context_id: variant.intervention?.context_id,
      }));
      row.append(measured);
      renderTimelineReplayTests(row, call, variants, replays);
    }
    const context = contextPatternsByCall.get(JSON.stringify([run.id, call.id]));
    if (context) showTimelineContext(row, context);
    appendTimelineChanges(row, transitionsByCall.get(contextCallKey({ run_id: run.id, call_id: call.id })));
    const button = node("button", call.retrieval?.label || call.decisions.map((decision) => decision.label).join(", ") || "Inspect call");
    button.addEventListener("click", () => openDecision(runIndex, index));
    row.append(button);
    list.append(row);
  }
}

function renderTimelineReplayTests(row, call, variants, replays) {
  const details = node("details", undefined, "timeline-changes timeline-replay-tests");
  details.append(node("summary", "Tested inputs and effects"));
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    const list = node("div", undefined, "timeline-replay-results");
    list.append(node("p", "Change in how often the replay matched the recorded decision", "muted"));
    for (const { report, variant } of variants) {
      const ids = variant.intervention ? [variant.intervention.context_id] :
        report.variants.map(({ intervention }) => intervention.context_id);
      const names = ids.map((id) => {
        const item = call.context.find((candidate) => candidate.id === id);
        return [item.source_id || item.id, item.source_version].filter(Boolean).join(" · ");
      });
      const title = names.join(" + ") + (variant.intervention ? "" : " · Joint test");
      const decision = call.decisions.find((item) => item.id === report.location.decision_id);
      const item = node("div", undefined, "timeline-replay-result");
      const button = occurrenceButton(report.location, title, {
        replay_id: report.id, context_id: variant.intervention?.context_id,
      });
      const label = [decision.label, report.runner_id, replayMatchLabel(report.match_rule), replayBaselineLabel(report.location)];
      item.append(button, node("small", label.join(" · "), "anchor"), replayEffectValue(variant.difference_pp));
      appendReplayOutcomes(item, report.location, report.id, variant.intervention?.context_id);
      list.append(item);
    }
    for (const { location, id, control_comparison: control } of replays) {
      if (!control) continue;
      const decision = call.decisions.find((item) => item.id === location.decision_id);
      const item = node("div", undefined, "timeline-replay-result");
      item.append(
        occurrenceButton(location, "Target compared with negative control", {
          replay_id: id, control_context_id: control.control_context_id,
        }),
        node("small", decision.label + " · Target minus control match rate", "anchor"),
        replayEffectValue(control.target_minus_control_pp, control.target_minus_control_95_interval_pp)
      );
      appendReplayInterval(item, control.target_minus_control_95_interval_pp);
      list.append(item);
    }
    details.append(list);
  });
  row.append(details);
}

function showTimelineContext(row, context) {
  const details = node("details", undefined, "timeline-changes");
  details.append(node("summary", context.length + (context.length === 1 ? " context link" : " context links")));
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    for (const { group, item } of context) {
      const replay = contextReplay(item);
      const parts = [contextSourceName(group.source) + " → " + group.decision_label, contextRelationshipName(group.relationship)];
      if (replay.effects.length) parts.push(contextReplayStatus[replay.status]);
      if (contextPlanningStatus(item, replay.status) === "ready") parts.push("Ready to plan");
      details.append(occurrenceButton(item, parts.join(" · "), item));
    }
  });
  row.append(details);
}

function timestampNanos(value) {
  if (typeof value !== "string") return null;
  const milliseconds = Date.parse(value);
  if (!Number.isFinite(milliseconds)) return null;
  const fraction = value.match(/\.(\d+)(?:Z|[+-]\d{2}:?\d{2})?$/)?.[1] ?? "";
  return BigInt(milliseconds) * 1_000_000n + BigInt(fraction.padEnd(9, "0").slice(3, 9));
}

function timelineThreads() {
  const shown = contextTimelines.map((timeline) => {
    const run = data.runs[timeline.runs[0]];
    const key = run.conversation_id
      ? JSON.stringify([run.source, run.organization_id, run.user_id, run.conversation_id])
      : JSON.stringify([run.id]);
    return { ...timeline, key, label: run.conversation_id || run.id };
  });
  const occurrences = new Map();
  for (const { label } of shown) occurrences.set(label, (occurrences.get(label) || 0) + 1);
  return shown.map((thread) => {
    const run = data.runs[thread.runs[0]];
    const label = [thread.label];
    if (occurrences.get(thread.label) > 1) {
      label.push(run.organization_id, run.user_id, run.source, !run.conversation_id && "Trace " + run.id);
    }
    if (thread.runs.length > 1) label.push(thread.runs.length + " traces");
    return { ...thread, label: label.filter(Boolean).join(" · ") };
  });
}

function patternScopes() {
  const groups = new Map();
  groups.set(defaultPatternScope, { organization: null, user: null, label: "All imported runs" });
  for (const { organization_id: organization, user_id: user } of data.runs) {
    if (organization !== null) groups.set(JSON.stringify([organization, null]), {
      organization, user: null, label: "Organization · " + organization,
    });
    if (user !== null) groups.set(JSON.stringify([organization, user]), {
      organization, user, label: "User · " + [organization, user].filter(Boolean).join(" / "),
    });
  }
  return groups;
}

function renderPatterns() {
  const scopes = patternScopes();
  const owner = element("owner");
  owner.replaceChildren(...[...scopes].map(([key, scope]) => new Option(scope.label, key)));
  owner.value = state.patternScope;
  owner.disabled = scopes.size < 2;
  const primaryScope = scopes.get(state.patternScope);
  const { organization, user } = primaryScope;
  const query = element("pattern-search").value.trim().toLowerCase();
  const list = element("pattern-list");
  list.replaceChildren();
  const patternType = element("pattern-type").value;
  element("activation-dates").hidden = patternType !== "activation";
  element("replay-pattern-dates").hidden = !isReplayPattern(patternType);
  element("source-profile-dates").hidden = patternType !== "sources";
  element("source-profile-evidence-field").hidden = patternType !== "sources";
  element("pattern-thread-filter").hidden = patternType === "tools" || patternType === "breakdown";
  element("breakdown-configuration").hidden = patternType !== "breakdown";
  element("visibility-pattern-filters").hidden = patternType !== "visibility";
  element("candidate-pattern-filters").hidden = !isAssessmentPattern(patternType);
  element("screening-relation-filter").hidden = patternType !== "screening";
  element("pattern-search-field").hidden = isAssessmentPattern(patternType) || ["breakdown", "visibility"].includes(patternType);
  element("pattern-search").placeholder = patternType === "sources" ? "Source, version, or trace type" : "Tool, source, or finding";
  element("context-replay-filter").hidden = patternType !== "context";
  element("context-view-filter").hidden = patternType !== "context";
  element("capture-filter").hidden = patternType !== "capture";
  element("pattern-compare-scope-filter").hidden = !supportsScopeComparison(patternType);
  element("pattern-compare-thread-filter").hidden = true;
  if (patternType === "breakdown") {
    state.patternThread = "";
    renderConflictBreakdown(list, primaryScope);
    return;
  }
  if (patternType === "visibility") {
    renderVisibilityPatterns(list, primaryScope, scopes);
    return;
  }
  if (isAssessmentPattern(patternType)) {
    renderAssessmentPatterns(list, primaryScope, scopes, patternType);
    return;
  }
  if (patternType === "context") {
    renderContextPatterns(list, organization, user, query);
    return;
  }
  if (isReplayPattern(patternType)) {
    renderReplayEffects(list, primaryScope, scopes, query, patternType === "controls");
    return;
  }
  if (patternType === "activation") {
    renderActivationPatterns(list, primaryScope, scopes, query);
    return;
  }
  if (patternType === "sources") {
    renderSourceProfiles(list, primaryScope, scopes, query);
    return;
  }
  if (patternType === "capture") {
    renderCapturePatterns(list, organization, user, query);
    return;
  }
  if (patternType === "changes") {
    renderTransitionPatterns(list, primaryScope, scopes, query);
    return;
  }
  if (patternType === "retrievals") {
    renderRetrievalPatterns(list, organization, user, query);
    return;
  }
  if (patternType === "tools") {
    renderToolPatterns(list, organization, user, query);
    return;
  }
  renderRulePatterns(list, organization, user, query);
}

function renderToolPatterns(list, organization, user, query) {
  for (const pattern of data.tool_patterns.filter((item) =>
    item.organization_id === organization && item.user_id === user
      && (item.tool + " " + item.source).toLowerCase().includes(query)
  )) {
    const card = node("article", undefined, "panel pattern");
    card.append(node("h2", pattern.tool));
    card.append(node("p", pattern.source, "muted"));
    const counts = [pattern.occurrences.length + " calls", pattern.run_count + " runs"];
    if (pattern.conversation_count !== null) counts.push(pattern.conversation_count + " conversations");
    if (pattern.turn_count !== null) counts.push(pattern.turn_count + " turns");
    card.append(node("p", counts.join(" · "), "count"));
    card.append(node("p", pattern.argument_matches + " calls with matching argument values", "muted"));
    const details = node("details");
    details.append(node("summary", "Open occurrences"));
    details.addEventListener("toggle", () => {
      if (details.open && details.childElementCount === 1) {
        for (const occurrence of pattern.occurrences) details.append(occurrenceButton(occurrence));
      }
    });
    card.append(details);
    list.append(card);
  }
  if (!list.children.length) list.append(node("p", query
    ? "No tool calls match this filter." : "No recorded tool calls in this scope.", "muted"));
}

function occurrenceButton(occurrence, text, reference) {
  const runIndex = data.runs.findIndex((run) => run.id === occurrence.run_id);
  const run = data.runs[runIndex];
  const callIndex = run.calls.findIndex((call) => call.id === occurrence.call_id);
  const decisionIndex = occurrence.decision_id === undefined ? 0 :
    run.calls[callIndex].decisions.findIndex((decision) => decision.id === occurrence.decision_id);
  const button = node("button", text || (run.conversation_id || run.id) + " / call " + (callIndex + 1), "occurrence");
  button.addEventListener("click", () => openDecision(runIndex, callIndex, decisionIndex, reference));
  return button;
}

function patternThreadKey(occurrence, group) {
  if (occurrence.conversation_id === null) return null;
  return JSON.stringify([occurrence.trace_source || group?.trace_source, occurrence.organization_id, occurrence.user_id, occurrence.conversation_id]);
}

function compareTimes(left, right) {
  return compareRecordedTimes(timestampNanos(left.started_at), timestampNanos(right.started_at));
}

function compareRecordedTimes(a, b) {
  if (a === null) return b === null ? 0 : 1;
  if (b === null) return -1;
  return a < b ? -1 : a > b ? 1 : 0;
}

function effectText(effect) {
  const display = (effect > 0 ? "+" : "") + effect.toFixed(1);
  return display + " pp";
}

function appendReplayInterval(container, interval, label) {
  if (!interval) return;
  const signed = (value) => (value > 0 ? "+" : "") + value;
  const range = signed(interval.lower) + " to " + signed(interval.upper) + " pp";
  const zero = interval.lower <= 0 && interval.upper >= 0 ? " · Includes zero" : "";
  const description = (label ? label + " · " : "") + "95% interval " + range + zero;
  container.append(node("small", description, "anchor replay-interval"));
}

function replayMatchLabel(rule) {
  switch (rule?.kind) {
    case "kind_and_label": return "Decision kind and label";
    case "outcome_fields": return "Outcome fields " + rule.pointers.join(", ");
    case "text_capture": return "Text capture " + rule.pattern;
    default: return "Exact decision";
  }
}

function effectRange(occurrences, field) {
  const measured = occurrences.map((item) => item[field]).filter((effect) => effect !== null);
  if (!measured.length) return "Unavailable";
  const [minimum, maximum] = measured.reduce(([min, max], effect) =>
    [Math.min(min, effect), Math.max(max, effect)], [Infinity, -Infinity]);
  return minimum === maximum ? effectText(minimum) : effectText(minimum) + " to " + effectText(maximum);
}

function scopedPatternGroups(patterns, organization, user) {
  return patterns.map((group) => ({
    ...group,
    occurrences: group.occurrences.filter((item) => inPatternScope(item, organization, user)),
  })).filter((group) => group.occurrences.length);
}

function inPatternScope(item, organization, user) {
  return (organization === null && user === null)
    || (item.organization_id === organization && (user === null || item.user_id === user));
}

function patternThreads(groups) {
  const threads = new Map();
  for (const group of groups) {
    for (const item of group.occurrences) {
      const key = patternThreadKey(item, group);
      if (key !== null) threads.set(key, [item.conversation_id, item.organization_id, item.user_id, item.trace_source || group.trace_source]
        .filter(Boolean).join(" · "));
    }
  }
  return threads;
}

function setPatternThreadOptions(select, threads, selection) {
  select.replaceChildren(new Option("All threads in scope", ""), ...[...threads].map(([key, label]) => new Option(label, key)));
  select.value = threads.has(selection) ? selection : "";
  select.disabled = threads.size === 0;
  return select.value;
}

function updatePatternThreadFilter(groups) {
  state.patternThread = setPatternThreadOptions(element("pattern-thread"), patternThreads(groups), state.patternThread);
}

function supportsScopeComparison(patternType) {
  return isAssessmentPattern(patternType) || isReplayPattern(patternType)
    || ["changes", "activation", "visibility", "sources"].includes(patternType);
}

function comparisonPatternFilter(scopes, patterns) {
  const scopeSelect = element("pattern-compare-scope");
  scopeSelect.replaceChildren(new Option("Single scope", ""),
    ...[...scopes].map(([key, scope]) => new Option(scope.label, key)));
  scopeSelect.value = patternComparison.scope;
  const scope = scopes.get(patternComparison.scope);
  if (!scope) return null;
  const groups = scopedPatternGroups(patterns, scope.organization, scope.user);
  const threads = patternThreads(groups);
  element("pattern-compare-thread-filter").hidden = false;
  patternComparison.thread = setPatternThreadOptions(element("pattern-compare-thread"), threads, patternComparison.thread);
  return { scope, groups, thread: patternComparison.thread };
}

function patternScopeCaption(scope, select) {
  if (!select.value) return scope.label;
  const [traceSource, organization, user, thread] = JSON.parse(select.value);
  return [scope.label, "Thread " + thread, organization !== scope.organization && organization,
    user !== scope.user && user, traceSource].filter(Boolean).join(" · ");
}

function patternScopeColumn(index) {
  const column = node("section", undefined, "pattern-scope-column");
  column.append(node("h3", index ? "Scope B" : "Scope A"));
  return column;
}

function patternScopeColumns(captions, sides, renderSide, empty = "No comparable measurements") {
  const columns = node("div", undefined, "compare-grid pattern-scope-columns");
  sides.forEach((side, index) => {
    const column = patternScopeColumn(index);
    column.append(node("p", captions[index], "muted"));
    if (side) renderSide(column, side, index);
    else column.append(node("p", empty, "muted"));
    columns.append(column);
  });
  return columns;
}

function patternScopeSummary(title, scopes, counts, shared, className) {
  const panel = node("section", undefined, "panel pattern " + className);
  panel.append(node("h2", title));
  const columns = node("div", undefined, "compare-grid pattern-scope-columns");
  const threadSelects = ["pattern-thread", "pattern-compare-thread"];
  for (const [index, scope] of scopes.entries()) {
    const column = patternScopeColumn(index);
    column.append(node("small", patternScopeCaption(scope, element(threadSelects[index])), "anchor"), counts[index]);
    columns.append(column);
  }
  panel.append(columns, node("p", shared + " in both scopes", "muted"));
  return panel;
}

function contextSourceName(source) {
  switch (source.kind) {
    case "named": return [source.id, source.version].filter(Boolean).join(" · ");
    case "tool_definition": return "Tool definition";
    case "tool_result": return source.tool + " result";
    case "user_input": return "User input";
    case "captured_text": return [source.role, source.context_kind.replaceAll("_", " ")].filter(Boolean).join(" ");
  }
}

function contextRelationshipName(relationship) {
  switch (relationship.kind) {
    case "annotation": {
      const basis = relationship.basis;
      return basis[0].toUpperCase() + basis.slice(1) + " · " + relationship.relation;
    }
    case "tool_definition": return "Exact tool-name match";
    case "argument_match": return "Exact argument match " + relationship.argument_path;
    case "outcome_match": return relationship.outcome_path
      ? "Exact outcome field match " + relationship.outcome_path : "Exact numeric outcome match";
    case "outcome_identifier": return "Exact answer identifier match";
  }
}

function contextReplayKey(item) {
  return JSON.stringify([item.run_id, item.call_id, item.decision_id, item.context_id]);
}

function sourceItemKey({ run_id, call_id, context_id }) {
  return JSON.stringify([run_id, call_id, context_id]);
}

function contextReplay(item) {
  return replaysByContext.get(contextReplayKey(item)) || noContextReplay;
}

function appendContextReplayResults(container, replay, summary = contextReplayStatus[replay.status]) {
  if (!replay.effects.length) return;
  const details = node("details", undefined, "context-replay-results");
  details.append(node("summary", summary));
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    for (const { group, effect } of replay.effects) {
      const operation = group.pair ? "Pair test" :
        { remove_json: "Remove JSON", replace_json: "Replace JSON", replace_text_fragment: "Replace text fragment" }[group.operation]
        || "Replace text";
      const change = effect.difference_pp === null ? "Effect unavailable" : effectText(effect.difference_pp);
      const result = node("div", undefined, "context-replay-result");
      result.append(occurrenceButton(effect.location, operation + " · " + change, effect));
      const counts = "Baseline " + trialCounts(effect.baseline) + " · Variant " + trialCounts(effect.variant);
      const provenance = [counts, replayBaselineLabel(effect.location), group.runner_id, replayMatchLabel(group.match_rule)];
      if (effect.rescored_from) provenance.push("Rescored from " + effect.rescored_from);
      result.append(node("small", provenance.join(" · "), "anchor"));
      appendReplayInterval(result, effect.difference_95_interval_pp, group.pair ? "Joint change" : undefined);
      if (group.pair && effect.interaction_pp !== null) {
        result.append(node("small", "Interaction " + effectText(effect.interaction_pp), "anchor"));
      }
      appendReplayInterval(result, effect.interaction_95_interval_pp, "Interaction");
      appendReplayOutcomes(result, effect.location, effect.replay_id, effect.context_id);
      details.append(result);
    }
  });
  container.append(details);
}

function distinctLinkedDecisions(occurrences) {
  return new Set(occurrences.map((item) => JSON.stringify([item.run_id, item.call_id, item.decision_id]))).size;
}

function renderContextPatterns(list, organization, user, query) {
  const groups = scopedPatternGroups(data.context_patterns, organization, user);
  updatePatternThreadFilter(groups);
  const status = element("context-replay-status").value;
  const entries = [];
  for (const group of groups) {
    const occurrences = group.occurrences.filter((item) =>
      !state.patternThread || patternThreadKey(item, group) === state.patternThread
    );
    if (!occurrences.length) continue;
    const source = contextSourceName(group.source);
    const relationship = contextRelationshipName(group.relationship);
    const contextPath = group.relationship.context_path;
    const searchable = [source, relationship, group.decision_label, group.trace_source, contextPath]
      .filter(Boolean).join(" ").toLowerCase();
    if (!searchable.includes(query) && !searchable.replace(/_+/g, " ").includes(query)) continue;
    const matching = occurrences.filter((item) => matchesContextReplayFilter(item, status));
    entries.push({ group, occurrences, matching, source, relationship, decisions: distinctLinkedDecisions(occurrences) });
  }
  if (entries.length) {
    const scope = node("section", undefined, "panel pattern context-scope-planning");
    scope.append(node("h2", "Replay coverage for these links"));
    renderContextReplayCoverage(scope, entries.flatMap(({ occurrences }) => occurrences));
    list.append(scope);
  }
  const visible = entries.filter((entry) => entry.matching.length);
  if (!visible.length) {
    list.append(node("p", "No context links match this scope and filter.", "muted"));
    return;
  }
  if (element("context-view").value === "matrix") {
    renderContextMatrix(list, entries, status);
    return;
  }
  list.append(node("p", "Open a recorded match or annotation to inspect the evidence and prepare a replay.", "muted"));
  visible.sort((left, right) => right.decisions - left.decisions);
  for (const entry of visible) list.append(contextPatternCard(entry, status));
}

function contextPatternCard({ group, occurrences, matching, source, relationship, decisions }, status, showHeading = true) {
  const card = node("article", undefined, "panel pattern context-pattern");
  if (showHeading) card.append(node("h2", source + " → " + group.decision_label));
  card.append(node("p", relationship, "muted"));
  const contextPath = group.relationship.context_path;
  if (!showHeading && contextPath) card.append(node("small", "Path in decoded context " + contextPath, "anchor"));
  const counts = [countLabel(decisions, "decision")];
  const threads = occurrences.map((item) => patternThreadKey(item, group));
  if (threads.every(Boolean)) {
    counts.push(countLabel(new Set(threads).size, "thread"));
    if (occurrences.every((item) => item.turn_id !== null)) {
      const turns = new Set(occurrences.map((item, index) => JSON.stringify([threads[index], item.turn_id])));
      counts.push(countLabel(turns.size, "turn"));
    }
  }
  card.append(node("p", counts.join(" · "), "count"));
  renderContextReplayCoverage(card, occurrences);
  const details = node("details");
  details.append(node("summary", status ? "Open matching links (" + matching.length + ")" : "Open recorded links"));
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    details.append(node("small", group.trace_source, "anchor"));
    const fingerprint = group.source.text_sha256 || group.source.json_sha256;
    if (fingerprint) details.append(node("small", "Context SHA-256 " + fingerprint, "anchor"));
    if (showHeading && contextPath) details.append(node("small", "Path in decoded context " + contextPath, "anchor"));
    for (const item of matching.sort(compareTimes)) {
      const name = [item.organization_id, item.user_id, item.conversation_id || item.run_id, item.turn_id]
        .filter(Boolean).join(" / ");
      const entry = node("div", undefined, "context-pattern-occurrence");
      entry.append(occurrenceButton(item, name, item), conversationOccurrenceButton(item));
      entry.append(node("small", [item.started_at, item.call_id].filter(Boolean).join(" · "), "anchor"));
      const replay = contextReplay(item);
      if (replay.effects.length) appendContextReplayResults(entry, replay);
      else entry.append(node("small", contextReplayStatus[replay.status], "muted"));
      appendLinkedInputPlan(entry, item, replay);
      details.append(entry);
    }
  });
  card.append(details);
  return card;
}

const replayPatternDateFields = ["after", "before", "after_b", "before_b"];
const replayPatternDate = (field) => element("replay-pattern-" + field);
const isReplayPattern = (kind) => kind === "effects" || kind === "controls";

function setupReplayPatternDates() {
  element("replay-pattern-dates").addEventListener("change", () => {
    linkNotice();
    renderPatterns();
  });
}

function writeReplayPatternLink(params) {
  const fields = patternComparison.scope ? replayPatternDateFields : replayPatternDateFields.slice(0, 2);
  for (const field of fields) {
    const value = replayPatternDate(field).value;
    if (value) params.set(field, value);
  }
}

function restoreReplayPatternLink(params) {
  let valid = true;
  let dated = false;
  for (const field of replayPatternDateFields) {
    const enabled = !field.endsWith("_b") || params.has("scope_b");
    const value = enabled ? params.get(field) ?? "" : "";
    const input = replayPatternDate(field);
    input.value = value;
    valid &&= input.value === value && input.validity.valid;
    dated ||= Boolean(value);
  }
  element("replay-pattern-dates").open = dated;
  return valid;
}

function replayDateCaption(selection) {
  return [selection.after && "From " + selection.after.slice(0, 10),
    selection.before && "Before " + selection.before.slice(0, 10)].filter(Boolean).join(" · ");
}

function replayScopeCaption(scope, threadSelect, selection) {
  return [patternScopeCaption(scope, threadSelect), replayDateCaption(selection)].filter(Boolean).join(" · ");
}

function renderReplayEffects(list, primaryScope, scopes, query, controlled) {
  const { organization, user } = primaryScope;
  const patterns = data.replay_effects.flatMap((group, index) =>
    Boolean(group.control_edit) === controlled ? [{ ...group, index }] : []
  );
  const primary = scopedPatternGroups(patterns, organization, user);
  updatePatternThreadFilter(primary);
  const comparison = comparisonPatternFilter(scopes, patterns);
  element("replay-pattern-compare-dates").hidden = !comparison;
  const selection = patternScopeSelection(primaryScope, state.patternThread, replayPatternDateFields.slice(0, 2).map(replayPatternDate));
  const compare = comparison && patternScopeSelection(comparison.scope, comparison.thread,
    replayPatternDateFields.slice(2).map(replayPatternDate));
  const primaryDates = selection && replayDateCaption(selection);
  element("replay-pattern-dates").querySelector("summary").textContent = comparison ? "Date ranges"
    : primaryDates ? "Date range · " + primaryDates : "Date range";
  if (!selection || (comparison && !compare)) {
    list.append(node("p", "Enter complete UTC dates with Before later than From.", "muted"));
    return;
  }
  const cohorts = [replayScope(primary, state.patternThread, selection)];
  const captions = [replayScopeCaption(primaryScope, element("pattern-thread"), selection)];
  if (comparison) {
    cohorts.push(replayScope(comparison.groups, comparison.thread, compare));
    captions.push(replayScopeCaption(comparison.scope, element("pattern-compare-thread"), compare));
    if (controlled) list.append(controlComparisonSummary(cohorts, captions));
  }

  let shown = false;
  for (const group of patterns) {
    const occurrences = cohorts.map((cohort) => cohort.get(group.index) || []);
    const first = occurrences[0][0] ?? occurrences[1]?.[0];
    if (!first) continue;
    const changes = controlled ? [group.target_edit, group.control_edit] : group.pair || [group];
    const contextIds = controlled ? [first.target_context_id, first.control_context_id] : first.context_ids || [first.context_id];
    const titles = changes.map((change, index) => [change.source_id || contextIds[index],
      change.source_version].filter(Boolean).join(" · "));
    const title = titles.join(controlled ? " vs " : " + ");
    const matchLabel = replayMatchLabel(group.match_rule);
    if (![title, group.runner_id, group.model, group.decision_label, matchLabel]
      .filter(Boolean).join(" ").toLowerCase().includes(query)) continue;
    const card = node("article", undefined, "panel pattern effect-group");
    card.append(node("h2", title));
    const design = controlled ? "Target vs negative control" : group.pair ? "Pair test" : group.context_role;
    card.append(node("p", [design, group.decision_label, group.model, group.runner_id, matchLabel]
      .filter(Boolean).join(" · "), "muted"));
    if (group.sampling_design === "independent") card.append(node("small", "Independent sampling declared", "anchor"));
    const edits = replayPatternEdits(changes, titles, controlled, Boolean(group.pair));
    const querySpec = { version: 1, selection, ...(compare && { compare }), group: group.key };
    const queryDetails = node("details");
    queryDetails.append(node("summary", "Query JSON"), node("pre", JSON.stringify(querySpec, null, 2)));
    const renderSide = (column, results) => {
      appendReplayPatternEvidence(column, group, results);
      if (!controlled) appendRecurringOutcomes(column, results);
    };
    if (comparison) {
      const details = node("details");
      details.append(node("summary", controlled ? "Target and negative-control edits" : "Tested edits"), edits);
      card.append(details, patternScopeColumns(captions, occurrences, renderSide));
    } else {
      appendReplayPatternEvidence(card, group, occurrences[0], edits);
      if (controlled) {
        if (!state.patternThread) appendControlScopeBreakdown(card, occurrences[0], organization, user);
      } else appendRecurringOutcomes(card, occurrences[0]);
    }
    card.append(queryDetails);
    list.append(card);
    shown = true;
  }
  if (!shown) list.append(node("p", controlled
    ? "No recorded control comparisons match the selected scopes and filter."
    : "No recorded replay effects match this scope and filter.", "muted"));
}

function replayScope(groups, thread, selection) {
  const after = timestampNanos(selection.after);
  const before = timestampNanos(selection.before);
  return new Map(groups.map((group) => [group.index, group.occurrences
    .filter((item) => !thread || patternThreadKey(item, group) === thread)
    .map((item) => [item, item.started_at_unix_nanos === undefined ? null : BigInt(item.started_at_unix_nanos)])
    .filter(([, time]) => (after === null || (time !== null && time >= after))
      && (before === null || (time !== null && time < before)))
    .sort((a, b) => compareRecordedTimes(a[1], b[1]))
    .map(([item]) => item)]));
}

function controlComparisonSummary(cohorts, captions) {
  const decisions = [];
  const counts = cohorts.map((groups) => {
    const reports = [...groups.values()].flat();
    const locations = new Set(reports.map(({ location }) => replayDecisionKey(location)));
    decisions.push(locations);
    return node("p", countLabel(reports.length, "recorded comparison") + " across " +
      countLabel(locations.size, "decision"), "count");
  });
  const shared = [...decisions[1]].filter((key) => decisions[0].has(key)).length;
  const panel = node("section", undefined, "panel pattern control-comparison-coverage");
  panel.append(node("h2", "Controlled comparisons by scope"),
    patternScopeColumns(captions, counts, (column, count) => column.append(count)),
    node("p", countLabel(shared, "recorded decision") + " in both scopes", "muted"));
  return panel;
}

function replayPatternEdits(changes, titles, controlled, paired) {
  const edits = document.createDocumentFragment();
  for (const [index, change] of changes.entries()) {
    if (controlled) edits.append(node("h3", (index ? "Negative control · " : "Target · ") + titles[index]));
    else if (paired) edits.append(node("h3", titles[index]));
    edits.append(node("small", "Tested context SHA-256 " + change.context_sha256, "anchor"));
    showReplayEdit(edits, change);
  }
  return edits;
}

function appendReplayPatternEvidence(container, group, occurrences, edits) {
  if (!occurrences.length) {
    container.append(node("p", "No recorded comparisons for these edits", "muted"));
    return;
  }
  const controlled = Boolean(group.control_edit);
  const measured = occurrences.filter((item) => item.difference_pp !== null);
  const verified = occurrences.filter(({ location }) => hasRecordedRequest(location.run_id, location.call_id)).length;
  const decisions = replayResultsByDecision(occurrences);
  const conversations = new Set(occurrences.map((item) => patternThreadKey(item)).filter(Boolean));
  const count = controlled ? " comparisons measured · " + countLabel(decisions.size, "decision")
    : " tests measured · " + countLabel(decisions.size, "decision");
  let summary = measured.length + " of " + occurrences.length + count;
  if (group.pair) summary += " · Joint " + effectRange(occurrences, "difference_pp")
    + " · Interaction " + effectRange(occurrences, "interaction_pp");
  else if (measured.length) summary += " · " + effectRange(occurrences, "difference_pp");
  container.append(node("p", summary, "count"), node("p", countLabel(conversations.size, "identified conversation")
    + " · " + verified + " of " + occurrences.length + " baselines verified", "muted"));
  const details = node("details");
  const label = !edits ? "Open recorded decisions"
    : controlled ? "Both edits and recorded decisions" : "Intervention and recorded decisions";
  details.append(node("summary", label));
  if (edits) details.append(edits);
  const samples = node("div");
  details.append(samples);
  let selected = occurrences;
  function renderPage(offset, focus = false) {
    const table = replayEffectTable(selected.slice(offset, offset + 20), group);
    samples.replaceChildren(table);
    patternPager(samples, offset, offset + 20 < selected.length ? offset + 20 : null, (next) => renderPage(next, true));
    if (focus) {
      table.tabIndex = -1;
      table.focus();
    }
  }
  details.addEventListener("toggle", () => {
    if (details.open && !samples.children.length) renderPage(0);
  });
  container.append(replayDecisionBreakdown(decisions, (result) => {
    selected = result
      ? occurrences.filter(({ location }) => decisions.get(replayDecisionKey(location)) === result)
      : occurrences;
    renderPage(0);
    details.open = true;
  }), details);
}

function appendControlScopeBreakdown(card, occurrences, organization, user) {
  const level = user !== null ? "Thread" : organization !== null ? "User" : "Organization";
  const cohorts = new Map();
  for (const item of occurrences) {
    const key = user !== null ? patternThreadKey(item) : organization !== null ? item.user_id : item.organization_id;
    const name = user !== null ? [item.conversation_id, item.trace_source].filter(Boolean).join(" · ") : key;
    if (!cohorts.has(key)) cohorts.set(key, { name: key === null ? "No recorded " + level.toLowerCase() : name, items: [] });
    cohorts.get(key).items.push(item);
  }
  if (cohorts.size < 2) return;
  const details = node("details", undefined, "control-scope-breakdown");
  details.append(node("summary", "Decision results by " + level.toLowerCase()));
  const table = node("table", undefined, "effects-table");
  table.createCaption().textContent = "Counts of recorded decisions by target minus control match rate";
  const heading = table.createTHead().insertRow();
  for (const label of [level, ...replayDecisionResults.map(([, label]) => label)]) {
    const cell = node("th", label);
    cell.scope = "col";
    heading.append(cell);
  }
  const rows = table.createTBody();
  for (const { name, items } of cohorts.values()) {
    const counts = replayDecisionCounts(replayResultsByDecision(items));
    const row = rows.insertRow();
    const cell = node("th", name);
    cell.scope = "row";
    row.append(cell);
    for (const [result] of replayDecisionResults) row.insertCell().textContent = counts.get(result) || 0;
  }
  const scroll = node("div", undefined, "effects-scroll");
  scroll.append(table);
  details.append(scroll);
  card.append(details);
}

function replayDecisionKey({ run_id, call_id, decision_id }) {
  return JSON.stringify([run_id, call_id, decision_id]);
}

function replayResultsByDecision(occurrences) {
  const results = new Map();
  for (const { location, difference_pp } of occurrences) {
    const key = replayDecisionKey(location);
    const current = results.get(key);
    if (difference_pp === null) {
      if (!current) results.set(key, "unavailable");
      continue;
    }
    const result = difference_pp < 0 ? "lower" : difference_pp > 0 ? "higher" : "unchanged";
    results.set(key, !current || current === "unavailable" || current === result ? result : "mixed");
  }
  return results;
}

function replayDecisionCounts(decisions) {
  const counts = new Map();
  for (const result of decisions.values()) counts.set(result, (counts.get(result) || 0) + 1);
  return counts;
}

function replayDecisionBreakdown(decisions, select) {
  const counts = replayDecisionCounts(decisions);
  const breakdown = node("div", undefined, "replay-breakdown");
  const bar = node("div", undefined, "replay-breakdown-bar");
  bar.setAttribute("aria-hidden", "true");
  const filters = node("div", undefined, "replay-breakdown-filters");
  filters.setAttribute("role", "group");
  filters.setAttribute("aria-label", "Replay outcomes by recorded decision");
  for (const [result, label] of replayDecisionResults) {
    const count = counts.get(result);
    if (!count) continue;
    const segment = node("span");
    segment.dataset.result = result;
    segment.style.flexGrow = count;
    bar.append(segment);
    const button = node("button", label + " · " + count);
    button.dataset.result = result;
    button.setAttribute("aria-pressed", "false");
    button.addEventListener("click", () => {
      const enabled = button.getAttribute("aria-pressed") === "false";
      for (const control of filters.children) control.setAttribute("aria-pressed", String(control === button && enabled));
      select(enabled ? result : null);
    });
    filters.append(button);
  }
  breakdown.append(bar, filters);
  return breakdown;
}

function replayEffectValue(effect, interval) {
  const value = node("span", undefined, "replay-effect-value");
  if (effect === null) {
    value.textContent = "Unavailable";
    return value;
  }
  const track = node("span", undefined, "replay-effect-track");
  track.setAttribute("aria-hidden", "true");
  const marker = node("span", undefined, "replay-effect-marker");
  marker.dataset.result = effect < 0 ? "lower" : effect > 0 ? "higher" : "unchanged";
  marker.style.setProperty("--effect-width", Math.abs(effect) / 2 + "%");
  track.append(marker);
  if (interval) {
    const range = node("span", undefined, "replay-effect-range");
    range.style.left = (interval.lower + 100) / 2 + "%";
    range.style.width = (interval.upper - interval.lower) / 2 + "%";
    track.append(range);
  }
  value.append(track, node("span", effectText(effect)));
  return value;
}

function trialCounts(counts) {
  return counts.matched + "/" + (counts.completed - (counts.unscored || 0)) + trialGaps(counts);
}

function trialGaps(counts) {
  const failed = counts.attempted - counts.completed;
  return (failed ? " · " + failed + " failed" : "") + (counts.unscored ? " · " + counts.unscored + " unscored" : "");
}

function replayEffectTable(occurrences, group) {
  const paired = Boolean(group.pair);
  const controlled = Boolean(group.control_edit);
  const table = node("table", undefined, "effects-table");
  const caption = table.createCaption();
  caption.className = "replay-trial-counts";
  caption.textContent = "Matches out of scored trials";
  const headings = table.createTHead().insertRow();
  const columns = ["Decision", "Baseline"];
  columns.push(...(controlled ? ["Target", "Control", "Target minus control"]
    : [paired ? "Joint edit" : "Edited", "Match-rate change"]));
  if (paired) columns.push("Interaction");
  for (const heading of columns) {
    const cell = node("th", heading);
    cell.scope = "col";
    headings.append(cell);
  }
  const rows = table.createTBody();
  for (const item of occurrences) {
    const row = rows.insertRow();
    const decision = row.insertCell();
    const { run_id: run, call_id: call, decision_id: id } = item.location;
    const path = new URLSearchParams({ v: "1", view: "decisions", run, call, decision: id });
    writeReference(path, item);
    const link = patternEvidenceLink("/#" + path, item.conversation_id || run);
    link.className = "replay-evidence-link";
    decision.append(link);
    const owner = [item.organization_id, item.user_id].filter(Boolean).join(" / ");
    if (owner) decision.append(node("small", owner, "anchor"));
    const provenance = [item.started_at, item.turn_id, replayBaselineLabel(item.location), item.rescored_from && "Rescored from " + item.rescored_from]
      .filter(Boolean).join(" · ");
    if (provenance) decision.append(node("small", provenance, "anchor"));
    if (!controlled) appendReplayOutcomes(decision, item.location, item.replay_id, item.context_id);
    for (const counts of [item.baseline, item.variant]) {
      row.insertCell().textContent = trialCounts(counts);
    }
    if (controlled) row.insertCell().textContent = trialCounts(item.control);
    const effect = row.insertCell();
    effect.append(replayEffectValue(item.difference_pp, item.difference_95_interval_pp));
    appendReplayInterval(effect, item.difference_95_interval_pp);
    if (paired) {
      const interaction = row.insertCell();
      interaction.textContent = item.interaction_pp === null ? "Unavailable" : effectText(item.interaction_pp);
      appendReplayInterval(interaction, item.interaction_95_interval_pp);
    }
  }
  const scroll = node("div", undefined, "effects-scroll");
  scroll.setAttribute("role", "region");
  scroll.setAttribute("aria-label", "Contributing replays");
  scroll.append(table);
  return scroll;
}

function render() {
  for (const button of document.querySelectorAll("[data-view]")) {
    button.setAttribute("aria-pressed", String(button.dataset.view === state.view));
    element(button.dataset.view).hidden = button.dataset.view !== state.view;
  }
  element("copy-view-link").hidden = !data?.runs.length || state.view === "pending";
  if (state.view === "decisions") renderDecision();
  else if (state.view === "conversation") renderConversation();
  else if (state.view === "timeline") renderTimeline();
  else if (state.view === "compare") renderComparison();
  else if (state.view === "patterns") renderPatterns();
}

async function start() {
  setupPendingView();
  try {
    const response = await fetch("/api/inspection");
    if (!response.ok) throw new Error("HTTP " + response.status);
    data = await response.json();
    for (const check of data.preflight_checks || []) preflightChecksByCall.set(contextCallKey(check), check);
    for (const screen of data.context_screens || []) screensByCall.set(contextCallKey(screen), screen);
    for (const [probes, index] of [[data.candidate_probes, candidateProbesByCall], [data.shadow_probes, shadowProbesByCall]]) {
      for (const probe of probes || []) {
        const key = contextCallKey(probe);
        if (!index.has(key)) index.set(key, []);
        index.get(key).push(probe);
      }
    }
    for (const { run_id, call_id } of data.recorded_requests || []) {
      recordedRequests.add(JSON.stringify([run_id, call_id]));
    }
    if (data.live_otlp) {
      const refresh = element("refresh-traces");
      refresh.hidden = false;
      refresh.addEventListener("click", () => {
        if (data.runs.length) {
          window.history.replaceState(null, "", unresolvedInspectionLink ?? inspectionLink().href);
        }
        window.location.reload();
      });
    }
    if (!data.runs.length) {
      element("status").textContent = "Waiting for GenAI traces. Refresh after your exporter sends a batch.";
      element("status").hidden = !data.live_otlp;
      document.querySelectorAll("[data-view]:not([data-view='pending'])").forEach((button) => { button.disabled = true; });
      state.view = "pending";
      render();
      return;
    }
    indexRetrievals(data.runs, data.document_handoffs);
    indexFindings();
    indexReplayOutcomes();
    indexCaptureProfiles();
    indexContextHistory();
    indexSourceMasks(data.activation_reports || []);
    for (const item of data.context_replays) {
      const effects = item.effects.map(({ group_index, occurrence_index }) => {
        const group = data.replay_effects[group_index];
        return { group, effect: group.occurrences[occurrence_index] };
      });
      const measured = effects.find(({ group, effect }) => !group.pair && effect.difference_pp !== null);
      replaysByContext.set(contextReplayKey(item), { effects, status: measured ? "measured" : "unavailable" });
    }
    for (const group of data.context_patterns) {
      for (const item of group.occurrences) {
        const key = JSON.stringify([item.run_id, item.call_id]);
        if (!contextPatternsByCall.has(key)) contextPatternsByCall.set(key, []);
        contextPatternsByCall.get(key).push({ group, item });
      }
    }
    if (!data.patterns.length && data.context_patterns.length) element("pattern-type").value = "context";
    else if (!data.patterns.length && data.tool_patterns.length) element("pattern-type").value = "tools";
    const linked = data.tool_links.find((item) => {
      const run = data.runs.find((run) => run.id === item.run_id);
      const call = run.calls.find((call) => call.id === item.call_id);
      return run.source.startsWith("exgentic:") &&
        call.relationships.some((relationship) => relationship.decision_id === item.decision_id);
    }) || data.tool_links.find((item) => item.run_id.startsWith("exgentic:")) || data.tool_links[0];
    if (linked) {
      state.run = data.runs.findIndex((item) => item.id === linked.run_id);
      const run = data.runs[state.run];
      state.call = run.calls.findIndex((item) => item.id === linked.call_id);
      state.decision = run.calls[state.call].decisions.findIndex((item) => item.id === linked.decision_id);
    }
    state.patternScope = defaultPatternScope;
    setupConflictBreakdown();
    setupVisibilityPatterns();
    setupAssessmentPatterns();
    setupReplayPatternDates();
    setupSourceProfileFilters();
    const share = element("copy-view-link");
    share.hidden = false;
    share.addEventListener("click", () => copyInspectionLink());
    window.addEventListener("hashchange", restoreInspectionLink);
    element("decision-conversation").addEventListener("click", returnToConversation);
    document.querySelectorAll("[data-view]:not([data-view='pending'])").forEach((button) => {
      button.addEventListener("click", () => {
        if (button.dataset.view === "conversation") {
          returnToConversation();
          return;
        }
        if (button.dataset.view === "patterns") conversationReturn = undefined;
        linkNotice();
        state.reference = undefined;
        state.timelineCall = null;
        state.view = button.dataset.view;
        render();
      });
    });
    element("run").addEventListener("change", (event) => {
      linkNotice();
      state.run = Number(event.target.value);
      state.call = 0;
      state.decision = 0;
      state.reference = undefined;
      render();
    });
    element("call").addEventListener("change", (event) => {
      linkNotice();
      state.call = Number(event.target.value);
      state.decision = 0;
      state.reference = undefined;
      render();
    });
    element("decision").addEventListener("change", (event) => {
      linkNotice();
      state.decision = Number(event.target.value);
      state.reference = undefined;
      render();
    });
    element("owner").addEventListener("change", (event) => {
      if (!event.target.value) {
        event.target.value = state.patternScope;
        return;
      }
      state.patternScope = event.target.value;
      conflictGroup = null;
      visibilityPatternGroup = null;
      assessmentPatternGroup = null;
      linkNotice();
      renderPatterns();
    });
    for (const id of ["pattern-type", "context-replay-status", "context-view", "capture-status", "pattern-search"]) {
      element(id).addEventListener(id === "pattern-search" ? "input" : "change", () => {
        linkNotice();
        if (id === "pattern-type") assessmentPatternGroup = null;
        if (id === "pattern-search" && element("pattern-type").value === "sources") queueSourceProfileSearch();
        else renderPatterns();
      });
    }
    setupActivationDates();
    element("pattern-thread").addEventListener("change", (event) => {
      linkNotice();
      state.patternThread = event.target.value;
      visibilityPatternGroup = null;
      assessmentPatternGroup = null;
      renderPatterns();
    });
    element("pattern-compare-scope").addEventListener("change", (event) => {
      patternComparison.scope = event.target.value;
      patternComparison.thread = "";
      visibilityPatternGroup = null;
      assessmentPatternGroup = null;
      linkNotice();
      renderPatterns();
    });
    element("pattern-compare-thread").addEventListener("change", (event) => {
      patternComparison.thread = event.target.value;
      visibilityPatternGroup = null;
      assessmentPatternGroup = null;
      linkNotice();
      renderPatterns();
    });
    element("timeline-run").addEventListener("change", (event) => {
      conversationReturn = undefined;
      linkNotice();
      state.run = timelineThreads()[Number(event.target.value)].runs[0];
      state.call = 0;
      state.decision = 0;
      state.timelineCall = null;
      render();
    });
    element("timeline-layout").addEventListener("change", () => {
      linkNotice();
      renderTimeline();
      if (state.timelineCall) focusTimelineCall(state.timelineCall);
    });
    if (!restoreInspectionLink()) {
      selectConversationAnswer();
      render();
    }
    element("status").hidden = true;
  } catch (error) {
    element("status").textContent = "Could not load the evidence. " + error.message;
  }
}

document.addEventListener("DOMContentLoaded", start, { once: true });
