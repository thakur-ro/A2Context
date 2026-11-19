"use strict";

let assessmentPatternGroup = null;
const assessmentDates = ["after", "before", "after_b", "before_b"];
const assessmentField = (part) => element("candidate-pattern-" + part);
const isAssessmentPattern = (kind) => ["candidates", "shadow", "screening"].includes(kind);

function setupAssessmentPatterns() {
  for (const field of ["source", "recurring", "relation", ...assessmentDates]) {
    assessmentField(field).addEventListener("change", () => {
      assessmentPatternGroup = null;
      linkNotice();
      renderPatterns();
    });
  }
}

function openAssessmentPatterns(kind, run, sourceId) {
  state.view = "patterns";
  state.patternScope = JSON.stringify([run.organization_id, run.user_id]);
  state.patternThread = "";
  patternComparison.scope = "";
  patternComparison.thread = "";
  assessmentPatternGroup = null;
  element("pattern-type").value = kind;
  assessmentField("source").value = sourceId;
  assessmentField("relation").value = "";
  assessmentField("recurring").checked = false;
  for (const date of assessmentDates) assessmentField(date).value = "";
  linkNotice();
  render();
  assessmentField("source").focus();
}

function writeAssessmentPatternLink(params) {
  const fields = patternComparison.scope ? assessmentDates : assessmentDates.slice(0, 2);
  for (const field of ["source", ...fields]) {
    if (assessmentField(field).value) params.set(field, assessmentField(field).value);
  }
  if (assessmentField("recurring").checked) params.set("recurring", "1");
  const relation = assessmentField("relation").value;
  if (element("pattern-type").value === "screening" && relation) params.set("relation", relation);
  if (assessmentPatternGroup) params.set("group", assessmentPatternGroup);
}

function restoreAssessmentPatternLink(params) {
  assessmentPatternGroup = params.get("group");
  assessmentField("source").value = params.get("source") ?? "";
  assessmentField("recurring").checked = params.get("recurring") === "1";
  const relation = params.get("relation") ?? "";
  const filter = assessmentField("relation");
  filter.value = relation;
  let valid = filter.value === relation;
  if (!valid) filter.value = "";
  for (const date of assessmentDates) {
    const input = assessmentField(date);
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

function screeningPatternTitle(group) {
  const sources = group.sources.map((source) => [source.source_id || source.role || source.kind,
    source.source_version].filter(Boolean).join(" · "));
  return group.suggestions.map((kind) => screenLabels[kind]).join(" / ") + " · " + sources.join(" and ");
}

function describeScreeningGroup(card, group) {
  for (const excerpt of group.excerpts || []) {
    if (excerpt) {
      const preview = [...excerpt.text];
      card.append(node("p", preview.slice(0, 160).join("") + (preview.length > 160 || excerpt.truncated ? "…" : "")));
    }
  }
  if (group.scorer) card.append(node("p", [group.scorer.model_id, group.scorer.revision].join(" · "), "muted"));
}

function renderScreeningPatternStats(parent, stats) {
  const counts = [countLabel(stats.flagged_conversations, "identified conversation") + " with flags"];
  if (stats.flagged_organizations > 1) counts.push(countLabel(stats.flagged_organizations, "organization"));
  if (stats.unscored_calls) counts.push(countLabel(stats.unscored_calls, "call") + " without scores");
  parent.append(node("p", stats.flagged_calls + " of " + stats.assessed_calls + " assessed calls flagged"),
    node("p", counts.join(" · "), "muted"));
}

function assessmentPresentation(kind) {
  if (kind === "screening") return {
    title: "Screened context pairs", endpoint: "/api/screening-patterns", reports: data.context_screens,
    openLabel: "Open pair on turn", groupLabel: "source pair", empty: "No flagged pairs match this selection.",
    sources: ({ report }) => report.sources, groupTitle: screeningPatternTitle,
    describeGroup: describeScreeningGroup, renderStats: renderScreeningPatternStats,
    evidence: (item) => screenLabels[item.suggestion] || screenReasons[item.assessment.reason] || "No pair flagged",
  };
  const shadow = kind === "shadow";
  return {
    title: shadow ? "Shadow proposal shifts" : "Candidate preference shifts",
    endpoint: shadow ? "/api/shadow-patterns" : "/api/candidate-patterns",
    reports: shadow ? data.shadow_probes : data.candidate_probes,
    openLabel: "Open source on turn", groupLabel: "probe setup",
    empty: shadow ? "No shadow probe setups match this selection." : "No comparable candidate measurements match this selection.",
    sources: ({ measurement }) => measurement.effects,
    groupTitle: ({ setup }) => probePatternTitle(setup),
    describeGroup(card, { setup }) {
      const method = shadow ? [setup.runner_id, setup.match_rule.replaceAll("_", " ")] : [setup.model_id, setup.model_revision];
      card.append(node("p", [...method, setup.operation.replaceAll("_", " ")].join(" · "), "muted"));
      if (setup.target_excerpt) card.append(node("pre", setup.target_excerpt.text + (setup.target_excerpt.truncated ? "…" : "")));
    },
    renderStats: shadow ? renderShadowPatternStats : renderCandidatePatternStats,
    evidence: shadow ? shadowPatternEvidence : (item) => activationMargin.format(item.baseline_log_odds) + " → "
      + activationMargin.format(item.edited_log_odds) + " log odds" + (item.reversed ? " · Preference reversed" : ""),
  };
}

function renderAssessmentPatterns(list, scope, scopes, kind) {
  const view = assessmentPresentation(kind);
  const screening = kind === "screening";
  const runs = new Map(data.runs.map((run) => [run.id, run]));
  const sources = new Set();
  const occurrences = [];
  for (const probe of view.reports || []) {
    const run = runs.get(probe.run_id);
    if (!run || probe.status !== "matched") continue;
    const named = view.sources(probe).filter(({ source_id }) => source_id?.trim());
    if (!screening && !named.length) continue;
    for (const effect of named) sources.add(effect.source_id);
    occurrences.push({ ...run, trace_source: run.source });
  }
  element("candidate-sources").replaceChildren(...[...sources].sort().map((id) => new Option(id)));
  const available = [{ occurrences }];
  updatePatternThreadFilter(scopedPatternGroups(available, scope.organization, scope.user));
  const second = comparisonPatternFilter(scopes, available);
  assessmentField("compare-dates").hidden = !second;
  const selection = patternScopeSelection(scope, state.patternThread, assessmentDates.slice(0, 2).map(assessmentField));
  const compare = second && patternScopeSelection(second.scope, second.thread, assessmentDates.slice(2).map(assessmentField));
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
  const source = assessmentField("source").value.trim();
  if (source) spec.source_id = source;
  const relation = assessmentField("relation").value;
  if (screening && relation) spec.relation = relation;
  if (assessmentField("recurring").checked) spec.min_conversations = 2;
  const captions = second && [patternScopeCaption(scope, element("pattern-thread")),
    patternScopeCaption(second.scope, element("pattern-compare-thread"))];

  function renderEvidence(group, offset = 0, compareOffset = 0, focus = false) {
    assessmentPatternGroup = group;
    evidence.hidden = false;
    for (const button of results.querySelectorAll("[data-candidate-group]")) {
      button.setAttribute("aria-expanded", String(button.dataset.candidateGroup === group));
    }
    showPatternPage(evidence, view.endpoint, { ...spec, group, offset, compare_offset: compareOffset }, (page) => {
      const heading = node("h2", view.groupTitle(page.group));
      heading.tabIndex = -1;
      evidence.append(heading);
      if (screening) view.describeGroup(evidence, page.group);
      function renderSide(column, side, index) {
        for (const item of side.rows) {
          const row = node("article", undefined, "finding");
          const owner = [item.conversation_id || "Conversation ID unavailable", item.user_id, item.organization_id].filter(Boolean);
          row.append(node("h4", owner.join(" · ")),
            node("p", (item.started_at || "Date unavailable") + (item.turn_id ? " · " + item.turn_id : ""), "muted"),
            node("p", view.evidence(item)));
          const open = node("a", view.openLabel);
          open.href = item.inspector_path;
          row.append(open);
          column.append(row);
        }
        if (!side.rows.length) column.append(node("p", screening ? "No screened pairs on this page." : "No matching measurements on this page.", "muted"));
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
    showPatternPage(results, view.endpoint, { ...spec, offset }, ({ groups, total_groups, next_offset }) => {
      results.append(node("h2", view.title), node("p", countLabel(total_groups, view.groupLabel), "muted"));
      for (const group of groups) {
        const card = node("article", undefined, "finding");
        card.append(node("h3", view.groupTitle(group)));
        view.describeGroup(card, group);
        if (captions) card.append(patternScopeColumns(captions, [group.primary, group.comparison], view.renderStats,
          screening ? "No comparable screened pairs" : "No matching measurements"));
        else view.renderStats(card, group.primary);
        const inspect = node("button", "See contributing turns");
        inspect.dataset.candidateGroup = group.key;
        inspect.setAttribute("aria-controls", evidence.id);
        inspect.setAttribute("aria-expanded", String(group.key === assessmentPatternGroup));
        inspect.addEventListener("click", () => renderEvidence(group.key, 0, 0, true));
        card.append(inspect);
        results.append(card);
      }
      if (!groups.length) results.append(node("p", view.empty, "muted"));
      patternPager(results, offset, next_offset, renderGroups);
    });
  }
  renderGroups();
  if (assessmentPatternGroup) renderEvidence(assessmentPatternGroup);
}
