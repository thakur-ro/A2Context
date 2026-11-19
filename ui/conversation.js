"use strict";

const inlineTurnEvidence = matchMedia("(max-width: 760px)");
const conversationTime = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium", timeStyle: "short", timeZone: "UTC",
});
let conversationRows = [];
let conversationThreadKey;
let conversationEvidenceOpen = true;
let conversationReturn;
let renderedConversationEvidence;
const langSmithMessagePath = /^(inputs\.messages(?:\[0\])?)\[(0|[1-9]\d*)\](?:\.(?:kwargs|data))?\.content(?:\[(0|[1-9]\d*)\])?$/;
const openInferenceMessagePath = /^(llm\.input_messages)\.(0|[1-9]\d*)\.message\.(?:content|contents\.(0|[1-9]\d*)\.message_content\.text)$/;

function isUserText(item) {
  return item.role === "user" && ["message", "text", "message_part"].includes(item.kind);
}

function transcriptPosition({ field, span_id, message_index, part_index }, source) {
  let path;
  if (source === "langsmith" || source?.startsWith("langsmith:")) {
    path = langSmithMessagePath.exec(field);
  } else if (source === "otlp") {
    path = openInferenceMessagePath.exec(field);
  }
  if (path) {
    const [, stream, index, part] = path;
    return { stream, span_id, index: Number(index), part: part === undefined ? null : Number(part) };
  }
  return { stream: field, span_id, index: message_index, part: part_index };
}

function capturedUserMessage(items) {
  if (!items || items.some(({ item }) => item.role && item.role !== "user")) return "";
  const parts = items.filter(({ item }) => isUserText(item));
  if (parts.every(({ part }) => Number.isSafeInteger(part))) {
    parts.sort((left, right) => left.part - right.part);
  }
  return parts.map(({ item }) => item).filter(({ availability, text }) => availability === "included" && text?.trim())
    .map(({ text }) => text).join("\n");
}

function capturedUserInput(call, identifiedTurn, source) {
  const last = call.context.findLast(isUserText);
  if (!last) return null;
  const { stream, span_id, index: lastIndex } = transcriptPosition(last.evidence, source);
  if (!Number.isSafeInteger(lastIndex)) {
    const text = capturedUserMessage([{ item: last }]);
    return text ? { text, key: JSON.stringify([stream, lastIndex, text]) } : null;
  }
  const messages = new Map();
  let latest = lastIndex;
  for (const item of call.context) {
    const position = transcriptPosition(item.evidence, source);
    const { index, part } = position;
    if (position.stream !== stream || position.span_id !== span_id || !Number.isSafeInteger(index)) continue;
    if (!messages.has(index)) messages.set(index, []);
    messages.get(index).push({ item, part });
    if (isUserText(item)) latest = Math.max(latest, index);
  }
  const lastText = capturedUserMessage(messages.get(latest));
  if (!lastText) return null;
  const inputs = [lastText];
  if (identifiedTurn) {
    for (let index = latest - 1; messages.has(index); index--) {
      const text = capturedUserMessage(messages.get(index));
      if (!text) break;
      inputs.push(text);
    }
  }
  return { text: inputs.reverse().join("\n\n"), key: JSON.stringify([stream, latest, lastText]) };
}

function conversationTurns(calls) {
  const turns = [];
  const byId = new Map();
  const lastInputByRun = new Map();
  for (const entry of calls) {
    const { call, runIndex, run } = entry;
    const id = call.turn_id?.trim() ? call.turn_id : null;
    // A declared ID can span traces only within a recorded conversation.
    const key = id === null ? null : JSON.stringify([run.conversation_id ? null : runIndex, id]);
    let turn = key === null ? null : byId.get(key);
    if (!turn) {
      turn = { id, calls: [], input: null };
      turns.push(turn);
      if (key !== null) byId.set(key, turn);
    }
    turn.calls.push(entry);
    const input = capturedUserInput(call, id !== null, run.source);
    if (input) {
      if (!turn.input && (id !== null || lastInputByRun.get(runIndex) !== input.key)) turn.input = input.text;
      lastInputByRun.set(runIndex, input.key);
    }
  }
  return turns;
}

function recordedTurnSteps({ calls }) {
  return calls.flatMap(({ runIndex: run, index: call, call: record }) => {
    const steps = record.decisions.map((decision, index) => ({
      run, call, decision: index, record: decision, label: decision.label,
    }));
    return steps.length ? steps : [{ run, call, decision: 0, label: record.retrieval?.label || "Recorded call" }];
  });
}

function selectConversationAnswer() {
  const thread = contextTimelines.find(({ runs }) => runs.includes(state.run));
  const turn = conversationTurns(thread.calls).find(({ calls }) =>
    calls.some(({ runIndex, index }) => runIndex === state.run && index === state.call)
  );
  const answer = recordedTurnSteps(turn).findLast(({ record }) => record?.kind === "model_output");
  if (answer) selectConversationStep(answer);
}

function conversationSignal(turn, measuredSourceCalls, maskReplayCalls) {
  const links = new Set();
  let conflicts = 0;
  let preflightConflicts = 0;
  let changed = false;
  let visibility = false;
  let maskReplay = false;
  for (const { run, call } of turn.calls) {
    const key = contextCallKey({ run_id: run.id, call_id: call.id });
    const check = preflightChecksByCall.get(key);
    preflightConflicts += check?.report?.findings.filter(({ kind }) => kind === "conflict").length ?? 0;
    for (const { item } of contextPatternsByCall.get(key) || []) links.add(JSON.stringify([key, item.context_id]));
    conflicts += callFindings(run.id, call.id).filter(({ kind }) => kind === "conflict").length;
    changed ||= Boolean(transitionsByCall.get(key)?.changes.length);
    visibility ||= measuredSourceCalls.has(key);
    maskReplay ||= maskReplayCalls.has(key);
  }
  const measured = turn.calls.some(({ call, run }) =>
    data.replays.some(({ location, variants, pair }) => location.run_id === run.id && location.call_id === call.id
      && [...variants, pair].some((test) => test && test.difference_pp !== null))
  );
  return [
    preflightConflicts && countLabel(preflightConflicts, "preflight conflict"),
    conflicts && countLabel(conflicts, "conflict"), links.size && countLabel(links.size, "linked input"),
    maskReplay ? "Saved masked replays" : visibility && "Measured source effects",
    measured && "Measured replay", changed && "Context changed",
  ].filter(Boolean).join(" · ");
}

function conversationMessage(role, value) {
  const message = node("div", undefined, "conversation-message");
  message.append(node("p", role, "muted"), node("p", value));
  return message;
}

function renderConversation() {
  const threads = timelineThreads();
  const active = threads.findIndex(({ runs }) => runs.includes(state.run));
  const thread = threads[active];
  const picker = element("conversation-thread");
  options(picker, threads.map(({ label }) => label), active);
  element("conversation-picker").hidden = threads.length === 1;
  picker.onchange = () => {
    const { runIndex, index } = threads[Number(picker.value)].calls[0];
    openConversation(runIndex, index);
  };
  if (thread.key !== conversationThreadKey) {
    const run = data.runs[thread.runs[0]];
    element("conversation-title").textContent = run.conversation_id || run.id;
    const owner = element("conversation-owner");
    owner.textContent = [run.organization_id, run.user_id, run.source].filter(Boolean).join(" · ");
    owner.hidden = !owner.textContent;
    const rail = element("conversation-evidence-rail");
    const panel = element("conversation-evidence");
    if (panel.parentNode !== rail) rail.append(panel);
    const list = element("conversation-turns");
    list.replaceChildren();
    const visibilityReports = (data.activation_reports || []).filter(({ kind }) => kind === "source_visibility");
    const measuredSourceCalls = new Set(visibilityReports.map(({ location }) => contextCallKey(location)));
    const maskReplayCalls = new Set(visibilityReports.filter(({ continuations }) => continuations?.length)
      .map(({ location }) => contextCallKey(location)));
    let known = 0;
    conversationRows = conversationTurns(thread.calls).map((turn, position) => {
      const steps = recordedTurnSteps(turn);
      const title = turn.id === null ? "Call " + (position + 1) : "Turn " + ++known;
      const item = node("li", undefined, "conversation-turn");
      const time = turn.calls[0].call.started_at;
      const header = node("p", title, "conversation-turn-label muted");
      if (timestampNanos(time) !== null) {
        const timestamp = node("time", conversationTime.format(new Date(time)) + " UTC");
        timestamp.dateTime = time;
        header.append(" · ", timestamp);
      }
      if (turn.id === null) header.append(" · Turn ID unavailable");
      item.append(header);
      if (turn.input) item.append(conversationMessage(turn.id === null ? "Captured user input" : "User", turn.input));
      const answer = steps.findLast(({ record }) => record?.kind === "model_output");
      if (answer) {
        const alternatives = steps.filter((step) => step.run === answer.run && step.call === answer.call
          && step.record?.kind === "model_output");
        const text = alternatives.length > 1 ? countLabel(alternatives.length, "recorded answer")
          : answer.record.outcome === null ? "Answer content unavailable" : readable(answer.record.outcome);
        item.append(conversationMessage("Assistant", text));
      }
      const other = steps.filter(({ record }) => record?.kind !== "model_output");
      if (other.length === 1) {
        item.append(conversationMessage(labels[other[0].record?.kind] || "Recorded step", other[0].label));
      } else if (other.length) {
        item.append(node("p", countLabel(other.length, "recorded step"), "muted"));
      }
      const signal = conversationSignal(turn, measuredSourceCalls, maskReplayCalls);
      const button = node("button", signal || "Turn evidence", "conversation-turn-button");
      button.type = "button";
      button.setAttribute("aria-controls", "conversation-evidence");
      button.addEventListener("click", () => {
        const top = button.getBoundingClientRect().top;
        const selected = currentConversationRow() === row;
        if (!selected) selectConversationStep(answer || steps[0]);
        conversationEvidenceOpen = !selected || !conversationEvidenceOpen;
        renderTurnEvidence();
        if (inlineTurnEvidence.matches) window.scrollBy(0, button.getBoundingClientRect().top - top);
      });
      item.append(button);
      list.append(item);
      const row = { item, button, title, steps };
      return row;
    });
    conversationThreadKey = thread.key;
  }
  renderTurnEvidence();
}

function currentConversationRow() {
  return conversationRows.find(({ steps }) => steps.some(
    ({ run, call }) => run === state.run && call === state.call
  ));
}

function selectConversationStep({ run, call, decision }) {
  Object.assign(state, { run, call, decision, reference: undefined });
}

function openConversation(run, call, decision = 0) {
  linkNotice();
  conversationReturn = undefined;
  selectConversationStep({ run, call, decision });
  state.view = "conversation";
  conversationEvidenceOpen = true;
  render();
}

function conversationOccurrenceButton(occurrence) {
  const { call, runIndex, index } = contextCallIndex.get(contextCallKey(occurrence));
  const decision = call.decisions.findIndex(({ id }) => id === occurrence.decision_id);
  const button = node("button", "Open turn");
  button.type = "button";
  button.addEventListener("click", () => {
    openConversation(runIndex, index, Math.max(0, decision));
    currentConversationRow()?.button.focus();
  });
  return button;
}

function positionConversationEvidence() {
  const evidence = element("conversation-evidence");
  const row = currentConversationRow();
  const parent = inlineTurnEvidence.matches && row ? row.item : element("conversation-evidence-rail");
  if (parent !== evidence.parentNode) parent.append(evidence);
}

function evidenceSection(parent, label, open, populate) {
  const section = node("details", undefined, "conversation-evidence-section");
  section.append(node("summary", label));
  section.open = open;
  if (open) populate(section);
  else section.addEventListener("toggle", () => {
    if (section.open && section.childElementCount === 1) populate(section);
  });
  parent.append(section);
}

function renderRecordedPreflight(parent, run, call, open) {
  const check = preflightChecksByCall.get(contextCallKey({ run_id: run.id, call_id: call.id }));
  if (!check) return;
  const findings = check.report?.findings || [];
  const label = check.status === "matched"
    ? "Preflight · " + (findings.length ? countLabel(findings.length, "finding") : "no findings")
    : "Preflight · could not verify";
  evidenceSection(parent, label, open, (section) => {
    if (check.issue) {
      section.append(node("p", check.issue));
    } else {
      section.append(node("p", "The workflow reported this check before inference. Its request and rule sources match the recorded call.", "muted"));
      if (check.scope) section.append(node("small", check.scope.key + " · " + check.scope.scope, "anchor"));
      if (!findings.length) {
        section.append(node("p", countLabel(check.report.scoped_rule_items, "declared rule") + " checked."));
      }
      const inputs = new Map(call.context.map((item) => [item.id, item]));
      for (const finding of findings) {
        const card = node("article", undefined, "finding " + finding.kind);
        card.append(node("h3", [labels[finding.kind], finding.key, finding.scope].join(" · ")));
        for (const source of finding.sources) {
          const detail = contextDetails(inputs.get(source.context_id), false);
          detail.querySelector("summary").append(node("span", source.value === null ? "Value unavailable" : readable(source.value), "muted"));
          card.append(detail);
        }
        if (finding.kind === "conflict") {
          const recurring = node("button", "Find recurring preflight conflicts");
          recurring.addEventListener("click", () => openConflictBreakdown(run, finding.key, "preflight"));
          card.append(recurring);
        }
        section.append(card);
      }
    }
    section.append(evidence(check.evidence));
  });
}

function renderTurnEvidence() {
  const row = currentConversationRow();
  const panel = element("conversation-evidence");
  panel.hidden = !row || !conversationEvidenceOpen;
  for (const item of conversationRows) {
    item.button.setAttribute("aria-expanded", String(item === row && !panel.hidden));
  }
  if (panel.hidden) return;
  positionConversationEvidence();
  element("conversation-evidence-position").textContent = row.title + " evidence";
  const picker = element("conversation-step");
  const index = row.steps.findIndex(({ run, call, decision }) =>
    run === state.run && call === state.call && decision === state.decision);
  options(picker, row.steps.map(({ label, record }) => record && labels[record.kind]
    ? labels[record.kind] + " · " + label : label), index);
  element("conversation-step-picker").hidden = row.steps.length === 1;
  picker.onchange = () => {
    selectConversationStep(row.steps[Number(picker.value)]);
    renderConversationEvidence();
  };
  renderConversationEvidence();
}

function renderConversationEvidence() {
  const position = JSON.stringify([state.run, state.call, state.decision]);
  if (renderedConversationEvidence === position) return;
  renderedConversationEvidence = position;
  const run = data.runs[state.run];
  const call = run.calls[state.call];
  const decision = call.decisions[state.decision];
  const title = element("conversation-evidence-title");
  title.textContent = decision?.label || call.retrieval?.label || "Recorded call";
  const details = element("conversation-details");
  details.replaceChildren();
  const key = contextCallKey({ run_id: run.id, call_id: call.id });
  const links = (contextPatternsByCall.get(key) || []).filter(({ item }) => item.decision_id === decision?.id);
  const findings = decision ? callFindings(run.id, call.id, decision.id) : [];
  renderRecordedPreflight(details, run, call, true);
  if (decision && decision.kind !== "model_output") {
    evidenceSection(details, "Recorded outcome", !findings.length && !links.length, (section) => {
      section.append(node("pre", decision.outcome === null ? "Outcome unavailable" : readable(decision.outcome)));
      const linked = data.tool_links.find((item) => item.run_id === run.id && item.call_id === call.id
        && item.decision_id === decision.id);
      if (linked) {
        section.append(node("h3", "Recorded tool result"));
        section.append(node("pre", linked.result.outcome === null ? "Result content unavailable" : readable(linked.result.outcome)));
        section.append(occurrenceButton({ run_id: run.id, call_id: linked.result_call_id }, "Open call with tool result"));
      }
    });
  }
  if (findings.length) {
    evidenceSection(details, "Rule findings · " + findingCounts(findings).join(" · "), true, (section) => {
      renderFindingCards(section, run, call, decision, decision.outcome !== null);
      if (findings.some(({ kind }) => kind === "conflict")) {
        const recurring = node("button", "Find recurring conflicts");
        recurring.addEventListener("click", () => openConflictBreakdown(run, decision.applies_to.key));
        section.append(recurring);
      }
    });
  }
  if (links.length) {
    evidenceSection(details, "Linked · " + countLabel(links.length, "relationship"), !findings.length, (section) => {
      for (const { group, item } of links) {
        const { kind, index } = item.evidence;
        const link = kind === "annotation" ? call.relationships[index] : data.context_links[index];
        const row = node("div", undefined, "conversation-link");
        row.append(node("p", contextSourceName(group.source) + " · " + contextRelationshipName(group.relationship)));
        if (link.excerpt) row.append(node("blockquote", link.excerpt));
        row.append(occurrenceButton(item, "Open linked evidence", item));
        section.append(row);
      }
    });
  }
  const reports = activationReports(run, call, decision);
  const maskReports = reports.filter(({ kind, continuations }) => kind === "source_visibility" && continuations?.length);
  if (maskReports.length) {
    const count = maskReports.reduce((sum, report) => sum + report.continuations.length, 0);
    evidenceSection(details, "Masked replays · " + countLabel(count, "saved result"), true,
      (section) => renderTurnMaskReplays(section, maskReports, call));
  }
  if (decision) {
    renderDecisionInvestigation(details, run, call, decision, captureOnCall(run.id, call.id),
      exactDecisionLinks(run, call, decision));
  }
  const count = call.context.length;
  evidenceSection(details, "Present · " + count + (count === 1 ? " context entry" : " context entries"),
    !findings.length && !links.length && (!decision || decision.kind === "model_output"), (section) => {
    for (const item of call.context) section.append(contextDetails(item, false));
    if (!call.context.length) section.append(node("p", "Context capture unavailable.", "muted"));
  });
  if (decision && data.replays.some(({ location }) => location.run_id === run.id && location.call_id === call.id
    && location.decision_id === decision.id)) {
    evidenceSection(details, "Replay results", false, (section) => renderReplays(section, run, call, decision));
  }
  if (reports.length) {
    evidenceSection(details, "Activation probe", !maskReports.length, (section) => renderActivationReports(section, reports, call));
  }
  appendTimelineChanges(details, transitionsByCall.get(key));
  const inspect = node("button", "Open decision details");
  inspect.type = "button";
  inspect.addEventListener("click", () => openDecision(state.run, state.call, state.decision));
  details.append(inspect);
}

function renderTurnMaskReplays(section, reports, call) {
  const names = new Map(call.context.map(({ id, source_id }) => [id, source_id || id.replaceAll("-", " ")]));
  const results = node("div", undefined, "turn-mask-replays");
  const reportPicker = node("select");
  options(reportPicker, reports.map(({ model_id, id }) => model_id + " · " + id), 0);
  if (reports.length > 1) {
    const label = node("label", "Saved probe ", "activation-picker");
    label.append(reportPicker);
    section.append(label);
  }
  section.append(results);

  function load(offset = 0) {
    const report = reports[Number(reportPicker.value)];
    showPatternPage(results, "/api/source-visibility", {
      version: 1, location: report.location, report_id: report.id, view: { kind: "continuations" }, offset, limit: 20,
    }, (page) => {
      const list = node("ul");
      for (const row of page.rows) {
        const sources = row.context_ids.map((id) => names.get(id)).join(" + ");
        const token = row.start_output_token + 1;
        const item = node("li");
        item.append(node("strong", sources), node("small", maskOutcomeLabels[row.outcome] + " · Token " + token, "muted"));
        const open = occurrenceButton(report.location, "Open replay", {
          activation_id: report.id, visibility_token: row.start_output_token, visibility_continuation: row.continuation_index,
        });
        open.setAttribute("aria-label", "Open replay for " + sources + " from token " + token);
        item.append(open);
        list.append(item);
      }
      results.append(list);
      patternPager(results, offset, page.next_offset, load);
    });
  }
  reportPicker.addEventListener("change", () => load());
  load();
}

function returnToConversation() {
  const position = conversationReturn || state;
  conversationReturn = undefined;
  openConversation(position.run, position.call, position.decision);
  currentConversationRow()?.button.focus({ preventScroll: position.scroll !== undefined });
  if (position.scroll !== undefined) window.scrollTo(0, position.scroll);
}

inlineTurnEvidence.addEventListener("change", () => {
  if (state.view === "conversation" && conversationRows.length) positionConversationEvidence();
});
