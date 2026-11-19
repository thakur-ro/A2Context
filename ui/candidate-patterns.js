"use strict";

let candidatePatternGroup = null;
const candidateDates = ["after", "before", "after_b", "before_b"];
const candidateField = (part) => element("candidate-pattern-" + part);

function setupCandidatePatterns() {
  for (const field of ["source", "recurring", ...candidateDates]) {
    candidateField(field).addEventListener("change", () => {
      candidatePatternGroup = null;
      linkNotice();
      renderPatterns();
    });
  }
}

function openCandidatePatterns(run, sourceId) {
  state.view = "patterns";
  state.patternScope = JSON.stringify([run.organization_id, run.user_id]);
  state.patternThread = "";
  patternComparison.scope = "";
  patternComparison.thread = "";
  candidatePatternGroup = null;
  element("pattern-type").value = "candidates";
  candidateField("source").value = sourceId;
  candidateField("recurring").checked = false;
  for (const date of candidateDates) candidateField(date).value = "";
  linkNotice();
  render();
  candidateField("source").focus();
}

function writeCandidatePatternLink(params) {
  const fields = patternComparison.scope ? candidateDates : candidateDates.slice(0, 2);
  for (const field of ["source", ...fields]) {
    if (candidateField(field).value) params.set(field, candidateField(field).value);
  }
  if (candidateField("recurring").checked) params.set("recurring", "1");
  if (candidatePatternGroup) params.set("group", candidatePatternGroup);
}

function restoreCandidatePatternLink(params) {
  candidatePatternGroup = params.get("group");
  candidateField("source").value = params.get("source") ?? "";
  candidateField("recurring").checked = params.get("recurring") === "1";
  let valid = true;
  for (const date of candidateDates) {
    const input = candidateField(date);
    const value = params.get(date) ?? "";
    input.value = value;
    valid &&= input.value === value && input.validity.valid;
  }
  return valid;
}

function candidatePatternTitle({ source_id, source_version, candidate_ids }) {
  return [source_id, source_version, candidate_ids.join(" vs ")].filter(Boolean).join(" · ");
}

function renderCandidatePatternStats(parent, stats) {
  const range = stats.min_change_log_odds === stats.max_change_log_odds
    ? activationMargin.format(stats.min_change_log_odds)
    : activationMargin.format(stats.min_change_log_odds) + " to " + activationMargin.format(stats.max_change_log_odds);
  parent.append(node("p", stats.reversals + " of " + stats.measurements + " edits reversed preference · " + range + " log odds"),
    node("p", [countLabel(stats.calls, "call"), countLabel(stats.conversations, "identified conversation"),
      countLabel(stats.organizations, "organization")].join(" · "), "muted"));
}

function renderCandidatePatterns(list, scope, scopes) {
  const runs = new Map(data.runs.map((run) => [run.id, run]));
  const sources = new Set();
  const occurrences = [];
  for (const probe of data.candidate_probes || []) {
    const run = runs.get(probe.run_id);
    if (!run || probe.status !== "matched") continue;
    const named = probe.measurement.effects.filter(({ source_id }) => source_id?.trim());
    if (!named.length) continue;
    for (const effect of named) sources.add(effect.source_id);
    occurrences.push({ ...run, trace_source: run.source });
  }
  element("candidate-sources").replaceChildren(...[...sources].sort().map((id) => new Option(id)));
  const available = [{ occurrences }];
  updatePatternThreadFilter(scopedPatternGroups(available, scope.organization, scope.user));
  const second = comparisonPatternFilter(scopes, available);
  candidateField("compare-dates").hidden = !second;
  const selection = patternScopeSelection(scope, state.patternThread, candidateDates.slice(0, 2).map(candidateField));
  const compare = second && patternScopeSelection(second.scope, second.thread, candidateDates.slice(2).map(candidateField));
  const results = node("section", undefined, "panel visibility-pattern-results");
  const evidence = node("section", undefined, "panel breakdown-evidence");
  evidence.id = "candidate-pattern-evidence";
  evidence.hidden = true;
  list.append(results, evidence);
  if (!selection || (second && !compare)) {
    results.append(node("p", "Enter complete dates in order from earliest to latest.", "muted"));
    return;
  }

  const spec = { version: 1, selection, limit: 20 };
  if (compare) spec.compare = compare;
  const source = candidateField("source").value.trim();
  if (source) spec.source_id = source;
  if (candidateField("recurring").checked) spec.min_conversations = 2;
  const captions = second && [patternScopeCaption(scope, element("pattern-thread")),
    patternScopeCaption(second.scope, element("pattern-compare-thread"))];

  function renderEvidence(group, offset = 0, compareOffset = 0, focus = false) {
    candidatePatternGroup = group;
    evidence.hidden = false;
    for (const button of results.querySelectorAll("[data-candidate-group]")) {
      button.setAttribute("aria-expanded", String(button.dataset.candidateGroup === group));
    }
    showPatternPage(evidence, "/api/candidate-patterns", { ...spec, group, offset, compare_offset: compareOffset }, (page) => {
      const heading = node("h2", candidatePatternTitle(page.group.setup));
      heading.tabIndex = -1;
      evidence.append(heading);
      function renderSide(column, side, index) {
        for (const item of side.rows) {
          const row = node("article", undefined, "finding");
          const owner = [item.conversation_id || "Conversation ID unavailable", item.user_id, item.organization_id].filter(Boolean);
          row.append(node("h4", owner.join(" · ")),
            node("p", (item.started_at || "Date unavailable") + (item.turn_id ? " · " + item.turn_id : ""), "muted"),
            node("p", activationMargin.format(item.baseline_log_odds) + " → " + activationMargin.format(item.edited_log_odds)
              + " log odds" + (item.reversed ? " · Preference reversed" : "")));
          const open = node("a", "Open source on turn");
          open.href = item.inspector_path;
          row.append(open);
          column.append(row);
        }
        if (!side.rows.length) column.append(node("p", "No matching measurements on this page.", "muted"));
        patternPager(column, index ? compareOffset : offset, side.next_offset, (next) => {
          renderEvidence(group, index ? offset : next, index ? next : compareOffset, true);
        });
      }
      if (captions) evidence.append(patternScopeColumns(captions, [page.primary, page.comparison], renderSide));
      else renderSide(evidence, page.primary, 0);
      if (focus) heading.focus();
    });
  }

  function renderGroups(offset = 0) {
    showPatternPage(results, "/api/candidate-patterns", { ...spec, offset }, ({ groups, total_groups, next_offset }) => {
      results.append(node("h2", "Candidate preference shifts"), node("p", countLabel(total_groups, "probe setup"), "muted"));
      for (const group of groups) {
        const card = node("article", undefined, "finding");
        card.append(node("h3", candidatePatternTitle(group.setup)), node("p", [group.setup.model_id,
          group.setup.model_revision, group.setup.operation.replaceAll("_", " ")].join(" · "), "muted"));
        if (captions) card.append(patternScopeColumns(captions, [group.primary, group.comparison], renderCandidatePatternStats));
        else renderCandidatePatternStats(card, group.primary);
        const inspect = node("button", "See contributing turns");
        inspect.dataset.candidateGroup = group.key;
        inspect.setAttribute("aria-controls", evidence.id);
        inspect.setAttribute("aria-expanded", String(group.key === candidatePatternGroup));
        inspect.addEventListener("click", () => renderEvidence(group.key, 0, 0, true));
        card.append(inspect);
        results.append(card);
      }
      if (!groups.length) results.append(node("p", "No comparable candidate measurements match this selection.", "muted"));
      patternPager(results, offset, next_offset, renderGroups);
    });
  }
  renderGroups();
  if (candidatePatternGroup) renderEvidence(candidatePatternGroup);
}
