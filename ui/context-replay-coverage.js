"use strict";

function linkedInputRequest(occurrence) {
  const { run_id, call_id, decision_id, context_id } = occurrence;
  const { run, call, contextItems } = contextCallIndex.get(contextCallKey(occurrence));
  const decision = call.decisions.find((entry) => entry.id === decision_id);
  const item = contextItems.get(context_id);
  const capture = captureOnCall(run_id, call_id);
  return { run, call, decision, request: replayRequestStatus(decision, item, capture) };
}

function contextPlanningStatus(occurrence, replayStatus) {
  if (replayStatus === "measured") return "measured";
  return readyToPlanReplay(linkedInputRequest(occurrence).request) ? "ready" : "gaps";
}

function matchesContextReplayFilter(occurrence, filter) {
  if (!filter) return true;
  const { status } = contextReplay(occurrence);
  if (filter === "ready" || filter === "gaps") return contextPlanningStatus(occurrence, status) === filter;
  return status === filter;
}

function contextReplayCoverage(occurrences) {
  const unique = new Map(occurrences.map((item) => [contextReplayKey(item), item]));
  const counts = { measured: 0, unavailable: 0, ready: 0, gaps: 0, total: unique.size };
  for (const item of unique.values()) {
    const { status } = contextReplay(item);
    counts[contextPlanningStatus(item, status)] += 1;
    if (status === "unavailable") counts.unavailable += 1;
  }
  return counts;
}

function planningCoverageLabel({ ready, gaps }) {
  return ready + " ready to plan · " + gaps + " with planning gaps";
}

function renderContextReplayCoverage(card, occurrences) {
  const counts = contextReplayCoverage(occurrences);
  const { measured, unavailable, total } = counts;
  const coverage = node("p", undefined, "context-replay-coverage");
  const meter = node("meter");
  meter.max = total;
  meter.value = measured;
  meter.setAttribute("aria-label", "Linked contexts with an individual replay measurement");
  const summary = measured + " of " + total + " linked contexts measured";
  coverage.append(meter, node("span", unavailable ? summary + " · " + unavailable + " effect unavailable" : summary));
  card.append(coverage);
  if (measured < total) card.append(node("p", planningCoverageLabel(counts), "muted"));
}

function appendLinkedInputPlan(container, occurrence, replay) {
  if (replay.status === "measured") return;
  const { run, call, decision, request } = linkedInputRequest(occurrence);
  container.append(node("small", investigationRequests[request], "anchor"));
  if (readyToPlanReplay(request)) {
    const plan = node("button", "Plan replay");
    plan.type = "button";
    plan.addEventListener("click", () => openReplayPlanner(run, call, decision, [occurrence.context_id]));
    container.append(plan);
  }
}
