"use strict";

const defaultPatternScope = JSON.stringify([null, null]);
let unresolvedInspectionLink;

function linkNotice(message = "") {
  if (!message) unresolvedInspectionLink = undefined;
  const notice = element("link-status");
  notice.textContent = message;
  notice.hidden = !message;
}

function unavailableLink(message) {
  unresolvedInspectionLink = window.location.href;
  linkNotice(message);
}

function decisionLocation(position) {
  const run = data.runs[position.run];
  const call = run.calls[position.call];
  return { run: run.id, call: call.id, decision: call.decisions[position.decision]?.id };
}

function writeLocation(params, position, prefix = "") {
  for (const [key, value] of Object.entries(decisionLocation(position))) {
    if (value !== undefined) params.set(prefix + key, value);
  }
}

function readLocation(params, prefix = "", requireDecision = false) {
  const run = data.runs.findIndex((item) => item.id === params.get(prefix + "run"));
  if (run < 0) return null;
  const call = data.runs[run].calls.findIndex((item) => item.id === params.get(prefix + "call"));
  if (call < 0) return null;
  const id = params.get(prefix + "decision");
  if (id === null) return requireDecision ? null : { run, call, decision: 0 };
  const decision = data.runs[run].calls[call].decisions.findIndex((item) => item.id === id);
  return decision < 0 ? null : { run, call, decision };
}

function findingLocator(finding) {
  return JSON.stringify([finding.kind, finding.key, finding.scope, ...finding.context_ids.slice().sort()]);
}

function relationshipLocator(kind, item) {
  const key = kind === "annotation"
    ? [item.context_id, item.relation, item.basis, item.evidence?.span_id, item.evidence?.field,
      item.evidence?.message_index, item.evidence?.part_index]
    : [item.context_id, item.kind, item.argument_path, item.context_path];
  if (kind === "exact_match" && item.outcome_path !== undefined) key.push(item.outcome_path);
  return JSON.stringify(key);
}

function writeReference(params, reference, position) {
  if (!reference) return;
  if (reference.context_item_id !== undefined) params.set("context", reference.context_item_id);
  else if (reference.retrieved_document_id !== undefined) params.set("document", reference.retrieved_document_id);
  else if (reference.capture_profile) params.set("capture", "1");
  else if (reference.finding_index !== undefined) {
    params.set("finding", findingLocator(data.findings[reference.finding_index]));
  } else if (reference.evidence) {
    const { kind, index } = reference.evidence;
    const call = data.runs[position.run].calls[position.call];
    const item = kind === "annotation" ? call.relationships[index] : data.context_links[index];
    params.set(kind, relationshipLocator(kind, item));
  } else if (reference.replay_id !== undefined) {
    params.set("replay", reference.replay_id);
    if (reference.control_context_id !== undefined) params.set("control", reference.control_context_id);
    else if (reference.context_id !== undefined) params.set("variant", reference.context_id);
  } else if (reference.activation_id !== undefined) {
    params.set("activation", reference.activation_id);
    if (reference.visibility_token !== undefined) {
      params.set("visibility", reference.visibility_token);
      if (reference.visibility_pair !== undefined) params.set("pair", reference.visibility_pair);
      if (reference.visibility_source !== undefined) params.set("mask", reference.visibility_source);
      if (reference.visibility_continuation !== undefined) params.set("continuation", reference.visibility_continuation);
      return;
    }
    if (reference.output_token !== undefined) params.set("token", reference.output_token);
    else if (reference.branch_index !== undefined) params.set("branch", reference.branch_index);
    else params.set("checkpoint", reference.checkpoint_index);
    params.set("block", reference.block);
  }
}

function readReference(params, position) {
  if (params.has("context")) return { context_item_id: params.get("context") };
  if (params.has("document")) return { retrieved_document_id: params.get("document") };
  if (params.has("capture")) return { capture_profile: true };
  if (params.has("activation")) {
    const keys = ["token", "branch", "checkpoint", "visibility"].filter((key) => params.has(key));
    if (keys.length !== 1) return null;
    const [key] = keys;
    const index = params.get(key);
    if (!/^\d+$/.test(index || "")) return null;
    if (key === "visibility") {
      const pair = params.get("pair");
      const source = params.get("mask");
      const continuation = params.get("continuation");
      if (params.has("block") || (pair !== null && (!/^\d+$/.test(pair) || source !== null)) || source === ""
        || (continuation !== null && (!/^\d+$/.test(continuation) || pair !== null || source !== null))) return null;
      return {
        activation_id: params.get("activation"), visibility_token: Number(index),
        visibility_pair: pair === null ? undefined : Number(pair),
        visibility_source: source ?? undefined,
        visibility_continuation: continuation === null ? undefined : Number(continuation),
      };
    }
    if (params.has("pair") || params.has("mask") || params.has("continuation") || !/^\d+$/.test(params.get("block") || "")) return null;
    const fields = { token: "output_token", branch: "branch_index", checkpoint: "checkpoint_index" };
    return {
      activation_id: params.get("activation"),
      [fields[key]]: Number(index),
      block: Number(params.get("block")),
    };
  }
  if (params.has("replay")) {
    if (params.has("control")) return { replay_id: params.get("replay"), control_context_id: params.get("control") };
    return { replay_id: params.get("replay"), context_id: params.get("variant") ?? undefined };
  }
  const location = decisionLocation(position);
  if (params.has("finding")) {
    const finding = callFindings(location.run, location.call, location.decision)
      .find((item) => findingLocator(item) === params.get("finding"));
    return finding ? { finding_index: finding.finding_index } : null;
  }
  for (const kind of ["annotation", "exact_match"]) {
    if (!params.has(kind)) continue;
    const items = kind === "annotation" ? data.runs[position.run].calls[position.call].relationships : data.context_links;
    const matches = items.flatMap((item, index) => {
      const onDecision = item.decision_id === location.decision &&
        (kind === "annotation" || (item.run_id === location.run && item.call_id === location.call));
      return onDecision && relationshipLocator(kind, item) === params.get(kind) ? [index] : [];
    });
    return matches.length === 1 ? { evidence: { kind, index: matches[0] } } : null;
  }
}

function selectLinkValue(id, value) {
  const select = element(id);
  if ([...select.options].some((option) => option.value === value)) select.value = value;
}

function inspectionLink(reference = state.reference) {
  const url = new URL(window.location.pathname, window.location.href);
  const params = new URLSearchParams({ v: "1", view: state.view });
  switch (state.view) {
    case "conversation":
      writeLocation(params, state);
      if (state.probeLink) {
        const { kind, report, source } = state.probeLink;
        params.set(kind, report);
        params.set(kind + "_source", source);
      } else if (state.preflightSource) params.set("preflight_source", state.preflightSource);
      else if (state.screenPair) params.set("screen_pair", state.screenPair);
      if (reference) {
        params.set("view", "decisions");
        writeReference(params, reference, state);
      }
      break;
    case "decisions":
      writeLocation(params, state);
      writeReference(params, reference, state);
      break;
    case "timeline": {
      const thread = timelineThreads().find((item) => item.runs.includes(state.run));
      params.set("thread", thread.key);
      params.set("layout", element("timeline-layout-control").hidden ? "chronological" : element("timeline-layout").value);
      if (state.timelineCall) writeLocation(params, state.timelineCall);
      break;
    }
    case "patterns": {
      const kind = element("pattern-type").value;
      params.set("pattern", kind);
      if (state.patternScope !== defaultPatternScope) params.set("scope", state.patternScope);
      if (kind !== "tools" && state.patternThread) params.set("thread", state.patternThread);
      if (supportsScopeComparison(kind) && patternComparison.scope) {
        params.set("scope_b", patternComparison.scope);
        if (patternComparison.thread) params.set("thread_b", patternComparison.thread);
      }
      if (kind === "breakdown") writeConflictBreakdownLink(params);
      else if (kind === "visibility") writeVisibilityPatternLink(params);
      else if (isAssessmentPattern(kind)) writeAssessmentPatternLink(params);
      else if (element("pattern-search").value) params.set("q", element("pattern-search").value);
      if (kind === "sources") writeSourceProfileLink(params);
      if (kind === "context") {
        params.set("presentation", element("context-view").value);
        params.set("replay_status", element("context-replay-status").value);
      }
      if (kind === "capture") params.set("capture_status", element("capture-status").value);
      if (kind === "activation") {
        params.set("block", activationPatternBlock);
        writeActivationDateLink(params);
      }
      break;
    }
    case "compare":
      if (comparison[0]) writeLocation(params, comparison[0], "a_");
      if (comparison[1]) writeLocation(params, comparison[1], "b_");
      params.set("filter", element("compare-filter").value);
      break;
  }
  url.hash = params.toString();
  return url;
}

function patternEvidenceLink(path, label) {
  const link = node("a", label);
  link.href = path;
  link.addEventListener("click", (event) => {
    if (event.button === 0 && !event.ctrlKey && !event.metaKey && !event.shiftKey && !event.altKey) {
      window.history.replaceState(null, "", inspectionLink().href);
    }
  });
  return link;
}

async function copyInspectionLink(reference) {
  const url = inspectionLink(reference).href;
  try {
    await navigator.clipboard.writeText(url);
    linkNotice("Inspector link copied.");
  } catch {
    const dialog = element("copy-link-dialog");
    const input = element("share-url");
    input.value = url;
    if (!dialog.open) dialog.showModal();
    input.focus();
    input.select();
  }
}

function evidenceLinkButton(reference, label) {
  const button = node("button", "Copy link", "evidence-link");
  button.type = "button";
  button.setAttribute("aria-label", "Copy link to " + label);
  button.addEventListener("click", () => copyInspectionLink(reference));
  return button;
}

function restorePatternLink(params) {
  const scopes = patternScopes();
  const scope = params.get("scope") ?? defaultPatternScope;
  if (!scopes.has(scope)) {
    unavailableLink("The linked organization or user scope is unavailable in this inspector.");
    return false;
  }
  const thread = params.get("thread") ?? "";
  state.view = "patterns";
  state.patternScope = scope;
  state.patternThread = thread;
  selectLinkValue("pattern-type", params.get("pattern"));
  const kind = element("pattern-type").value;
  if (kind === "breakdown") restoreConflictBreakdownLink(params);
  const activation = kind === "activation";
  if (activation) activationPatternBlock = Number(params.get("block")) || 0;
  const datesAvailable = activation ? restoreActivationDateLink(params)
    : kind === "sources" ? restoreSourceProfileLink(params)
    : isAssessmentPattern(kind) ? restoreAssessmentPatternLink(params)
    : kind !== "visibility" || restoreVisibilityPatternLink(params);
  const compare = supportsScopeComparison(element("pattern-type").value);
  const comparisonScope = compare ? params.get("scope_b") ?? "" : "";
  const comparisonThread = compare ? params.get("thread_b") ?? "" : "";
  const comparisonAvailable = !comparisonScope || scopes.has(comparisonScope);
  patternComparison.scope = comparisonAvailable ? comparisonScope : "";
  patternComparison.thread = patternComparison.scope ? comparisonThread : "";
  element("pattern-search").value = params.get("q") ?? "";
  selectLinkValue("context-view", params.get("presentation") ?? "list");
  selectLinkValue("context-replay-status", params.get("replay_status") ?? "");
  selectLinkValue("capture-status", params.get("capture_status") ?? "");
  render();
  if (!datesAvailable) {
    unavailableLink(activation ? "The linked activation dates are invalid." : "The linked comparison dates are invalid.");
    element("pattern-list").replaceChildren();
  } else if (thread !== state.patternThread) {
    unavailableLink("The linked thread is unavailable for this pattern and scope.");
    element("pattern-list").replaceChildren();
  } else if (!comparisonAvailable || comparisonThread !== patternComparison.thread) {
    unavailableLink("The linked comparison scope or thread is unavailable in this inspector.");
    element("pattern-list").replaceChildren();
  }
  return true;
}

function restoreInspectionLink() {
  linkNotice();
  const params = new URLSearchParams(window.location.hash.slice(1));
  if (!params.size) return false;
  if (params.get("v") !== "1") {
    linkNotice("This inspector link uses an unsupported format.");
    return false;
  }
  conversationReturn = undefined;
  switch (params.get("view")) {
    case "conversation": {
      const position = readLocation(params);
      if (!position) {
        unavailableLink("The linked conversation call or decision is unavailable in this inspector.");
        return false;
      }
      openConversation(position.run, position.call, position.decision);
      const probe = ["probe", "shadow"].find((kind) => params.has(kind) || params.has(kind + "_source"));
      if (probe) {
        if (!focusProbeSource(probe, params.get(probe), params.get(probe + "_source"))) {
          unavailableLink("The linked probe source is unavailable on this call.");
        }
      } else if (params.has("preflight_source")) {
        if (!focusPreflightSource(params.get("preflight_source"))) {
          unavailableLink("The linked preflight conflict is unavailable on this call.");
        }
      } else if (params.has("screen_pair")) {
        if (!focusScreenPair(params.get("screen_pair"))) {
          unavailableLink("The linked screened pair is unavailable on this call.");
        }
      } else currentConversationRow()?.button.focus();
      return true;
    }
    case "decisions": {
      const position = readLocation(params);
      if (!position) {
        unavailableLink("The linked run, call, or decision is unavailable in this inspector.");
        return false;
      }
      const reference = readReference(params, position);
      state.view = "decisions";
      const found = openDecision(position.run, position.call, position.decision, reference || undefined);
      if (reference === null || !found) {
        unavailableLink("The linked evidence is unavailable on this call.");
      }
      return true;
    }
    case "timeline": {
      const thread = timelineThreads().find((item) => item.key === params.get("thread"));
      if (!thread) {
        unavailableLink("The linked timeline is unavailable in this inspector.");
        return false;
      }
      const anchor = readLocation(params);
      state.timelineCall = anchor && thread.runs.includes(anchor.run) ? anchor : null;
      state.view = "timeline";
      state.run = state.timelineCall?.run ?? thread.runs[0];
      state.call = state.timelineCall?.call ?? 0;
      state.decision = state.timelineCall?.decision ?? 0;
      selectLinkValue("timeline-layout", params.get("layout"));
      render();
      if (state.timelineCall) focusTimelineCall(state.timelineCall);
      if (!state.timelineCall && params.has("run")) {
        unavailableLink("The linked call is unavailable on this timeline.");
      } else if (params.get("layout") === "execution" && element("timeline-tree").hidden) {
        unavailableLink("Recorded spans for the linked execution tree are unavailable.");
      }
      return true;
    }
    case "patterns":
      return restorePatternLink(params);
    case "compare": {
      comparison[0] = readLocation(params, "a_", true);
      comparison[1] = readLocation(params, "b_", true);
      comparisonInitialized = true;
      state.view = "compare";
      selectLinkValue("compare-filter", params.get("filter") ?? "");
      render();
      if (compareSides.some((side, index) => params.has(side + "_run") && !comparison[index])) {
        unavailableLink("One or more linked decisions are unavailable in this inspector.");
      }
      return true;
    }
    default:
      linkNotice("The linked inspector view is unavailable.");
      return false;
  }
}
