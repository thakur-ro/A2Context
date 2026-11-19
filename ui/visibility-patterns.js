"use strict";

let visibilityPatternGroup = null;
const visibilityPatternDateFields = ["after", "before"];
const visibilityComparisonDateFields = ["after_b", "before_b"];
const visibilityEffectFormat = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 5, signDisplay: "exceptZero",
});
const visibilityField = (part) => element("visibility-pattern-" + part);

function setupVisibilityPatterns() {
  for (const field of ["kind", "token", "source", "repeat", ...visibilityPatternDateFields, ...visibilityComparisonDateFields]) {
    visibilityField(field).addEventListener("change", () => {
      visibilityPatternGroup = null;
      linkNotice();
      renderPatterns();
    });
  }
}

function openVisibilityPatterns(report, token, kind) {
  const run = data.runs.find((item) => item.id === report.location.run_id);
  state.view = "patterns";
  state.patternScope = JSON.stringify([run.organization_id, run.user_id]);
  state.patternThread = "";
  patternComparison.scope = "";
  patternComparison.thread = "";
  visibilityPatternGroup = null;
  element("pattern-type").value = "visibility";
  visibilityField("kind").value = kind;
  visibilityField("token").value = String(token + 1);
  visibilityField("source").value = "";
  visibilityField("repeat").checked = false;
  for (const date of [...visibilityPatternDateFields, ...visibilityComparisonDateFields]) visibilityField(date).value = "";
  linkNotice();
  render();
  visibilityField("token").focus();
}

function writeVisibilityPatternLink(params) {
  for (const field of ["kind", "token", "source", ...visibilityPatternDateFields]) {
    if (visibilityField(field).value) params.set(field, visibilityField(field).value);
  }
  if (patternComparison.scope) {
    for (const field of visibilityComparisonDateFields) {
      if (visibilityField(field).value) params.set(field, visibilityField(field).value);
    }
  }
  if (visibilityField("repeat").checked) params.set("recurring", "1");
  if (visibilityPatternGroup) params.set("group", visibilityPatternGroup);
}

function restoreVisibilityPatternLink(params) {
  visibilityPatternGroup = params.get("group");
  visibilityField("kind").value = params.get("kind") === "pair" ? "pair" : "source";
  visibilityField("token").value = params.get("token") ?? "1";
  visibilityField("repeat").checked = params.get("recurring") === "1";
  const source = params.get("source") ?? "";
  visibilityField("source").replaceChildren(new Option("All named sources", ""));
  if (source) visibilityField("source").add(new Option(source, source, true, true));
  let datesValid = true;
  for (const date of [...visibilityPatternDateFields, ...visibilityComparisonDateFields]) {
    const value = params.get(date) ?? "";
    const input = visibilityField(date);
    input.value = value;
    datesValid &&= input.value === value && input.validity.valid;
  }
  return datesValid;
}

function visibilityReportsInScope({ organization, user }) {
  const runs = new Map(data.runs.filter((run) =>
    (organization === null && user === null)
    || (run.organization_id === organization && (user === null || run.user_id === user)))
    .map((run) => [run.id, run]));
  return (data.activation_reports || []).flatMap((report) => {
    const run = runs.get(report.location.run_id);
    if (!run || report.kind !== "source_visibility") return [];
    const call = run.calls.find(({ id }) => id === report.location.call_id);
    const { organization_id, user_id, conversation_id, source } = run;
    const occurrence = { trace_source: source, organization_id, user_id, conversation_id };
    return [{ report, call, occurrence }];
  });
}

function visibilityPatternSelection(scope, thread, [after, before]) {
  if (!after.validity.valid || !before.validity.valid || (after.value && before.value && after.value >= before.value)) return null;
  const selection = { scope: patternApiScope(scope) };
  if (thread) {
    const [trace_source, organization_id, user_id, conversation_id] = JSON.parse(thread);
    selection.thread = { trace_source, organization_id, user_id, conversation_id };
  }
  if (after.value) selection.after = after.value + "T00:00:00Z";
  if (before.value) selection.before = before.value + "T00:00:00Z";
  return selection;
}

function renderVisibilityPatterns(list, scope, scopes) {
  const entries = visibilityReportsInScope(scope);
  updatePatternThreadFilter([{ occurrences: entries.map(({ occurrence }) => occurrence) }]);
  const secondScope = scopes.get(patternComparison.scope);
  const secondEntries = secondScope ? visibilityReportsInScope(secondScope) : [];
  const second = comparisonPatternFilter(scopes, [{ occurrences: secondEntries.map(({ occurrence }) => occurrence) }]);
  visibilityField("compare-dates").hidden = !second;
  const tokenInput = visibilityField("token");
  const kind = visibilityField("kind").value;
  const select = visibilityField("source");
  const selected = select.value;
  const sources = new Set();
  for (const [records, thread] of [[entries, state.patternThread], [secondEntries, second?.thread]]) {
    for (const { report, call, occurrence } of records) {
      if (thread && patternThreadKey(occurrence) !== thread) continue;
      const measured = new Set(kind === "pair" ? (report.pairs || []).flatMap(({ context_ids }) => context_ids)
        : report.sources.filter(({ masked_log_probs }) => masked_log_probs).map(({ context_id }) => context_id));
      for (const context of call.context) {
        if (context.source_id?.trim() && measured.has(context.id)) sources.add(context.source_id);
      }
    }
  }
  if (selected) sources.add(selected);
  select.replaceChildren(new Option("All named sources", ""), ...[...sources].sort().map((source) => new Option(source, source)));
  select.value = selected;
  const panel = node("section", undefined, "panel visibility-pattern-results");
  list.append(panel);
  if (!tokenInput.validity.valid || !Number.isSafeInteger(tokenInput.valueAsNumber)) {
    panel.append(node("p", "Enter a positive whole number for the output token.", "muted"));
    return;
  }
  const primary = visibilityPatternSelection(scope, state.patternThread, visibilityPatternDateFields.map(visibilityField));
  const compare = second && visibilityPatternSelection(second.scope, second.thread, visibilityComparisonDateFields.map(visibilityField));
  if (!primary || (second && !compare)) {
    const side = second ? " for Scope " + (primary ? "B" : "A") : "";
    panel.append(node("p", "Enter complete dates" + side + ". The From date must precede the Before date.", "muted"));
    return;
  }
  const spec = { version: 1, kind, output_token: tokenInput.valueAsNumber - 1, ...primary, limit: 20 };
  if (compare) spec.compare = compare;
  if (selected) spec.source_id = selected;
  if (visibilityField("repeat").checked) spec.min_conversations = 2;
  const captions = second ? [patternScopeCaption(scope, element("pattern-thread")),
    patternScopeCaption(second.scope, element("pattern-compare-thread"))] : [];
  const results = node("div");
  const back = node("button", "All comparison groups");
  back.type = "button";
  panel.append(back, results);
  function load(group, offset = 0, focus = false, compareOffset = 0) {
    visibilityPatternGroup = group;
    back.hidden = !group;
    const request = { ...spec, offset, group };
    if (compare && group) request.compare_offset = compareOffset;
    showPatternPage(results, "/api/visibility-patterns", request, (page) => {
      const heading = node("h2", group ? visibilityGroupLabel(page.group) : "Effects at token " + (spec.output_token + 1));
      heading.tabIndex = -1;
      results.append(heading);
      if (compare) {
        renderVisibilityComparison(results, page, captions, group, offset, compareOffset, load);
      } else if (group) {
        appendVisibilityModel(results, page.group.comparison);
        appendVisibilityGroupSummary(results, page.group, page.metric);
        renderVisibilityOccurrences(results, page.occurrences);
      } else {
        renderVisibilityGroups(results, page, (key) => load(key, 0, true));
      }
      if (!compare || !group) patternPager(results, offset, page.next_offset, (next) => load(group, next, true));
      if (focus && state.view === "patterns") heading.focus();
    });
  }
  back.addEventListener("click", () => load(null, 0, true));
  load(visibilityPatternGroup);
}

function visibilityGroupLabel(group) {
  return group.comparison.sources.map(({ source_id, source_version }) =>
    [source_id, source_version].filter(Boolean).join(" · ")).join(" + ");
}

function appendVisibilityGroupSummary(parent, group, metric) {
  const { effect } = group;
  const number = visibilityEffectFormat.format;
  const range = effect.minimum === effect.maximum ? number(effect.minimum)
    : number(effect.minimum) + " to " + number(effect.maximum);
  const unit = metric === "interaction_log_units" ? "interaction log units" : "change in log probability";
  parent.append(node("p", range + " · " + unit, "count"));
  parent.append(node("p", [countLabel(group.conversation_count, "conversation"), countLabel(group.user_count, "user"),
    countLabel(group.organization_count, "organization"), countLabel(group.decision_count, "decision")].join(" · "), "muted"));
}

function appendVisibilityModel(parent, comparison) {
  const model = node("small", [comparison.model_id, comparison.runtime, "Prefix " + comparison.output_prefix_sha256.slice(0, 8)].join(" · "), "anchor");
  model.title = "Model revision " + comparison.model_revision;
  parent.append(model);
}

function renderVisibilityGroups(parent, page, openGroup) {
  if (!page.groups.length) {
    parent.append(node("p", "No comparable measurements match these filters.", "muted"));
  }
  appendVisibilityCoverage(parent, page.coverage);
  for (const group of page.groups) {
    const line = node("article", undefined, "visibility-pattern-row");
    const button = node("button", visibilityGroupLabel(group), "occurrence");
    button.type = "button";
    button.addEventListener("click", () => openGroup(group.key));
    line.append(button);
    appendVisibilityGroupSummary(line, group, page.metric);
    appendVisibilityModel(line, group.comparison);
    parent.append(line);
  }
}

function appendVisibilityCoverage(parent, coverage, scope = "") {
  const missing = coverage.measurements_without_source_id;
  if (missing) parent.append(node("p", countLabel(missing, "measurement") + scope
    + " could not be grouped because a source ID was unavailable.", "muted"));
}

function renderVisibilityComparison(parent, page, captions, key, offset, compareOffset, load) {
  appendVisibilityCoverage(parent, page.coverage_a, " in Scope A");
  appendVisibilityCoverage(parent, page.coverage_b, " in Scope B");
  if (key) {
    appendVisibilityModel(parent, page.group.comparison);
    parent.append(visibilityComparisonColumns(captions, [page.scope_a, page.scope_b], (column, evidence, index) => {
      appendVisibilityGroupSummary(column, evidence.group, page.metric);
      renderVisibilityOccurrences(column, evidence.occurrences);
      patternPager(column, index ? compareOffset : offset, evidence.next_offset, (next) => {
        load(key, index ? offset : next, true, index ? next : compareOffset);
      });
    }));
    return;
  }
  if (!page.groups.length) parent.append(node("p", "No comparable measurements match these filters.", "muted"));
  for (const group of page.groups) {
    const line = node("article", undefined, "visibility-pattern-row");
    const button = node("button", visibilityGroupLabel(group), "occurrence");
    button.type = "button";
    button.addEventListener("click", () => load(group.key, 0, true));
    line.append(button);
    appendVisibilityModel(line, group.comparison);
    line.append(visibilityComparisonColumns(captions, [group.scope_a, group.scope_b], (column, summary) => {
      appendVisibilityGroupSummary(column, summary, page.metric);
    }));
    parent.append(line);
  }
}

function visibilityComparisonColumns(captions, sides, renderSide) {
  const columns = node("div", undefined, "compare-grid pattern-scope-columns");
  sides.forEach((side, index) => {
    const column = patternScopeColumn(index);
    column.append(node("p", captions[index], "muted"));
    if (side) renderSide(column, side, index);
    else column.append(node("p", "No comparable measurements", "muted"));
    columns.append(column);
  });
  return columns;
}

function visibilityEvidenceLink(path, label) {
  const link = node("a", label);
  link.href = path;
  link.addEventListener("click", (event) => {
    if (event.button === 0 && !event.ctrlKey && !event.metaKey && !event.shiftKey && !event.altKey) {
      window.history.replaceState(null, "", inspectionLink().href);
    }
  });
  return link;
}

function renderVisibilityOccurrences(parent, occurrences) {
  for (const row of occurrences) {
    const line = node("article", undefined, "visibility-pattern-row");
    const label = row.conversation_id || row.location.run_id;
    line.append(node("h3", label));
    const recorded = [row.organization_id, row.user_id, row.turn_id].filter(Boolean);
    const details = node("p", recorded.length ? recorded.join(" · ") + " · " : undefined, "muted");
    const time = Date.parse(row.started_at);
    if (Number.isFinite(time)) {
      const label = node("time", conversationTime.format(new Date(time)) + " UTC");
      label.dateTime = row.started_at;
      details.append(label);
    } else {
      details.append("Time unavailable");
    }
    line.append(details);
    const links = node("div", undefined, "visibility-pattern-links");
    links.append(visibilityEvidenceLink(row.inspector_path, "Open token " + (row.output_token + 1)));
    row.source_inspector_paths.forEach((path, index) => links.append(visibilityEvidenceLink(path, row.context_ids[index])));
    line.append(node("strong", visibilityEffectFormat.format(row.effect)), links);
    parent.append(line);
  }
}
