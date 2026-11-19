"use strict";

const documentHandoffByItem = new Map();
const documentConsumers = new Map();
const retrievalGroups = new Map();
const handoffLabels = {
  ambiguous: "Multiple retrievals match",
  timing_unavailable: "Retrieval timing unavailable",
  content_unavailable: "Document content unavailable for comparison",
  content_differs: "Retrieved and input text differ",
};

function indexRetrievals(runs, handoffs) {
  for (const handoff of handoffs) {
    documentHandoffByItem.set(sourceItemKey(handoff), handoff);
    if (!handoff.origin) continue;
    const key = sourceItemKey({
      run_id: handoff.run_id, call_id: handoff.origin.call_id, context_id: handoff.origin.document_id,
    });
    if (!documentConsumers.has(key)) documentConsumers.set(key, []);
    documentConsumers.get(key).push(handoff);
  }
  for (const run of runs) {
    for (const call of run.calls) {
      if (!call.retrieval) continue;
      const { label, documents } = call.retrieval;
      const key = JSON.stringify([run.source, label]);
      if (!retrievalGroups.has(key)) retrievalGroups.set(key, { label, trace_source: run.source, occurrences: [] });
      retrievalGroups.get(key).occurrences.push({
        run_id: run.id, call_id: call.id, trace_source: run.source, started_at: call.started_at,
        organization_id: run.organization_id, user_id: run.user_id, conversation_id: run.conversation_id,
        documents,
      });
    }
  }
}

function consumers(runId, callId, itemId) {
  return documentConsumers.get(sourceItemKey({ run_id: runId, call_id: callId, context_id: itemId })) || [];
}

function renderDocumentHandoff(details, run, call, item) {
  const handoff = documentHandoffByItem.get(sourceItemKey({ run_id: run.id, call_id: call.id, context_id: item.id }));
  if (handoff?.origin) {
    details.append(occurrenceButton({ run_id: run.id, call_id: handoff.origin.call_id },
      "Open matching retrieval", { retrieved_document_id: handoff.origin.document_id }));
  } else {
    details.append(node("small", handoffLabels[handoff?.status] || "No matching retrieval recorded in this trace", "anchor"));
  }
}

function renderRetrieval(outcome, run, call) {
  const { label, documents } = call.retrieval;
  outcome.append(node("div", "Retrieval", "eyebrow"), node("h1", label));
  if (call.ended_at) outcome.append(node("small", "Finished " + call.ended_at, "anchor"));
  if (!documents.length) {
    outcome.append(node("p", "No retrieved documents captured", "muted"));
    return;
  }
  outcome.append(node("h2", countLabel(documents.length, "retrieved document")));
  for (const item of documents) {
    const details = contextDetails(item, false);
    details.dataset.retrievedDocumentId = item.id;
    const inputs = consumers(run.id, call.id, item.id);
    if (!inputs.length) details.append(node("small", "No matched input recorded in this trace", "anchor"));
    for (const handoff of inputs) {
      details.append(occurrenceButton(handoff, "Open matched input", { context_item_id: handoff.context_id }));
    }
    details.append(evidenceLinkButton({ retrieved_document_id: item.id }, "this retrieved document"));
    outcome.append(details);
  }
}

function retrievalCounts(occurrences) {
  let captured = 0;
  let matched = 0;
  for (const { run_id, call_id, documents } of occurrences) {
    captured += documents.length;
    matched += documents.filter((document) => consumers(run_id, call_id, document.id).length).length;
  }
  return countLabel(occurrences.length, "retrieval") + " · " + matched + " of " + captured + " captured documents matched an input";
}

function renderRetrievalPatterns(list, organization, user, query) {
  const groups = scopedPatternGroups([...retrievalGroups.values()], organization, user);
  updatePatternThreadFilter(groups);
  for (const group of groups) {
    const occurrences = group.occurrences.filter((item) =>
      (!state.patternThread || patternThreadKey(item, group) === state.patternThread)
      && ([group.label, group.trace_source].some((text) => text.toLowerCase().includes(query))
        || item.documents.some((document) => document.document_identity?.id.toLowerCase().includes(query)))
    );
    if (!occurrences.length) continue;
    const card = node("article", undefined, "panel pattern retrieval-pattern");
    card.append(node("h2", group.label), node("p", group.trace_source, "muted"),
      node("p", retrievalCounts(occurrences), "count"));
    const details = node("details");
    details.append(node("summary", "Open retrievals"));
    details.addEventListener("toggle", () => {
      if (!details.open || details.childElementCount > 1) return;
      for (const item of occurrences.sort(compareTimes)) {
        const name = [item.organization_id, item.user_id, item.conversation_id || item.run_id].filter(Boolean).join(" / ");
        const row = node("div", undefined, "context-pattern-occurrence");
        row.append(occurrenceButton(item, name), node("small", retrievalCounts([item]), "anchor"));
        details.append(row);
      }
    });
    card.append(details);
    list.append(card);
  }
  if (!list.children.length) list.append(node("p", "No retrievals match this scope and filter.", "muted"));
}
