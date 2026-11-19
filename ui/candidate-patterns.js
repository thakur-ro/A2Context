"use strict";

let probePatternGroup = null;
const probeDates = ["after", "before", "after_b", "before_b"];
const probeField = (part) => element("candidate-pattern-" + part);
const isProbePattern = (kind) => kind === "candidates" || kind === "shadow";

function setupProbePatterns() {
  for (const field of ["source", "recurring", ...probeDates]) {
    probeField(field).addEventListener("change", () => {
      probePatternGroup = null;
      linkNotice();
      renderPatterns();
    });
  }
}

function openProbePatterns(kind, run, sourceId) {
  state.view = "patterns";
  state.patternScope = JSON.stringify([run.organization_id, run.user_id]);
  state.patternThread = "";
  patternComparison.scope = "";
  patternComparison.thread = "";
  probePatternGroup = null;
  element("pattern-type").value = kind;
  probeField("source").value = sourceId;
  probeField("recurring").checked = false;
  for (const date of probeDates) probeField(date).value = "";
  linkNotice();
  render();
  probeField("source").focus();
}

function writeProbePatternLink(params) {
  const fields = patternComparison.scope ? probeDates : probeDates.slice(0, 2);
  for (const field of ["source", ...fields]) {
    if (probeField(field).value) params.set(field, probeField(field).value);
  }
  if (probeField("recurring").checked) params.set("recurring", "1");
  if (probePatternGroup) params.set("group", probePatternGroup);
}

function restoreProbePatternLink(params) {
  probePatternGroup = params.get("group");
  probeField("source").value = params.get("source") ?? "";
  probeField("recurring").checked = params.get("recurring") === "1";
  let valid = true;
  for (const date of probeDates) {
    const input = probeField(date);
    const value = params.get(date) ?? "";
    input.value = value;
    valid &&= input.value === value && input.validity.valid;
  }
  return valid;
}

function probePatternTitle({ source_id, source_version, candidate_ids, expected_label, expected_kind }) {
  return [source_id, source_version, candidate_ids?.join(" vs ") ?? (expected_label || expected_kind.replaceAll("_", " "))].filter(Boolean).join(" · ");
}

function renderCandidatePatternStats(parent, stats) {
  const range = stats.min_change_log_odds === stats.max_change_log_odds
    ? activationMargin.format(stats.min_change_log_odds)
    : activationMargin.format(stats.min_change_log_odds) + " to " + activationMargin.format(stats.max_change_log_odds);
  parent.append(node("p", stats.reversals + " of " + stats.measurements + " edits reversed preference · " + range + " log odds"),
    node("p", [countLabel(stats.calls, "call"), countLabel(stats.conversations, "identified conversation"),
      countLabel(stats.organizations, "organization")].join(" · "), "muted"));
}

function renderShadowPatternStats(parent, stats) {
  const range = stats.min_difference_pp == null ? "No measured shift" :
    (stats.min_difference_pp === stats.max_difference_pp ? activationMargin.format(stats.min_difference_pp) :
      activationMargin.format(stats.min_difference_pp) + " to " + activationMargin.format(stats.max_difference_pp)) + " pp";
  const counts = [stats.increased && stats.increased + " increased", stats.decreased && stats.decreased + " decreased",
    stats.unchanged && stats.unchanged + " unchanged", stats.unavailable && stats.unavailable + " unavailable"];
  parent.append(node("p", [range, ...counts].filter(Boolean).join(" · ")),
    node("p", [countLabel(stats.calls, "call"), countLabel(stats.measured_conversations, "conversation") + " with scored comparisons",
      countLabel(stats.organizations, "organization")].join(" · "), "muted"));
}

function shadowPatternEvidence({ baseline, edited, difference_pp, difference_95_interval_pp }) {
  return shadowTrialCounts(baseline) + " → " + shadowTrialCounts(edited) + " · " +
    shadowContrast(difference_pp, difference_95_interval_pp);
}

function renderProbePatterns(list, scope, scopes, kind) {
  const shadow = kind === "shadow";
  const endpoint = shadow ? "/api/shadow-patterns" : "/api/candidate-patterns";
  const title = shadow ? "Shadow proposal shifts" : "Candidate preference shifts";
  const renderStats = shadow ? renderShadowPatternStats : renderCandidatePatternStats;
  const runs = new Map(data.runs.map((run) => [run.id, run]));
  const sources = new Set();
  const occurrences = [];
  for (const probe of (shadow ? data.shadow_probes : data.candidate_probes) || []) {
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
  probeField("compare-dates").hidden = !second;
  const selection = patternScopeSelection(scope, state.patternThread, probeDates.slice(0, 2).map(probeField));
  const compare = second && patternScopeSelection(second.scope, second.thread, probeDates.slice(2).map(probeField));
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
  const source = probeField("source").value.trim();
  if (source) spec.source_id = source;
  if (probeField("recurring").checked) spec.min_conversations = 2;
  const captions = second && [patternScopeCaption(scope, element("pattern-thread")),
    patternScopeCaption(second.scope, element("pattern-compare-thread"))];

  function renderEvidence(group, offset = 0, compareOffset = 0, focus = false) {
    probePatternGroup = group;
    evidence.hidden = false;
    for (const button of results.querySelectorAll("[data-candidate-group]")) {
      button.setAttribute("aria-expanded", String(button.dataset.candidateGroup === group));
    }
    showPatternPage(evidence, endpoint, { ...spec, group, offset, compare_offset: compareOffset }, (page) => {
      const heading = node("h2", probePatternTitle(page.group.setup));
      heading.tabIndex = -1;
      evidence.append(heading);
      function renderSide(column, side, index) {
        for (const item of side.rows) {
          const row = node("article", undefined, "finding");
          const owner = [item.conversation_id || "Conversation ID unavailable", item.user_id, item.organization_id].filter(Boolean);
          row.append(node("h4", owner.join(" · ")),
            node("p", (item.started_at || "Date unavailable") + (item.turn_id ? " · " + item.turn_id : ""), "muted"),
            node("p", shadow ? shadowPatternEvidence(item) : activationMargin.format(item.baseline_log_odds) + " → "
              + activationMargin.format(item.edited_log_odds) + " log odds" + (item.reversed ? " · Preference reversed" : "")));
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
    showPatternPage(results, endpoint, { ...spec, offset }, ({ groups, total_groups, next_offset }) => {
      results.append(node("h2", title), node("p", countLabel(total_groups, "probe setup"), "muted"));
      for (const group of groups) {
        const card = node("article", undefined, "finding");
        const { setup } = group;
        const method = shadow ? [setup.runner_id, setup.match_rule.replaceAll("_", " ")] : [setup.model_id, setup.model_revision];
        card.append(node("h3", probePatternTitle(setup)),
          node("p", [...method, setup.operation.replaceAll("_", " ")].join(" · "), "muted"));
        if (setup.target_excerpt) card.append(node("pre", setup.target_excerpt.text + (setup.target_excerpt.truncated ? "…" : "")));
        if (captions) card.append(patternScopeColumns(captions, [group.primary, group.comparison], renderStats));
        else renderStats(card, group.primary);
        const inspect = node("button", "See contributing turns");
        inspect.dataset.candidateGroup = group.key;
        inspect.setAttribute("aria-controls", evidence.id);
        inspect.setAttribute("aria-expanded", String(group.key === probePatternGroup));
        inspect.addEventListener("click", () => renderEvidence(group.key, 0, 0, true));
        card.append(inspect);
        results.append(card);
      }
      if (!groups.length) results.append(node("p", shadow ? "No shadow probe setups match this selection." : "No comparable candidate measurements match this selection.", "muted"));
      patternPager(results, offset, next_offset, renderGroups);
    });
  }
  renderGroups();
  if (probePatternGroup) renderEvidence(probePatternGroup);
}
