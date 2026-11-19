"use strict";

let visibilityPatternGroup = null;
let visibilityPatternUnit = "";
let visibilityPatternPeriod = "";
const visibilityTimeUnits = new Map([["", "All occurrences"], ["day", "Day"], ["week", "Week"], ["month", "Month"]]);
const visibilityPatternDateFields = ["after", "before"];
const visibilityComparisonDateFields = ["after_b", "before_b"];
const visibilityToolFields = ["tool_name", "tool_name_b"];
const visibilityField = (part) => element("visibility-pattern-" + part);

function setupVisibilityPatterns() {
  for (const field of ["kind", "token", "target", "source", "order", "repeat",
    ...visibilityToolFields, ...visibilityPatternDateFields, ...visibilityComparisonDateFields]) {
    visibilityField(field).addEventListener("change", () => {
      visibilityPatternGroup = null;
      linkNotice();
      renderPatterns();
    });
  }
}

function openVisibilityPatterns(report, token, kind, target = "") {
  const run = data.runs.find((item) => item.id === report.location.run_id);
  state.view = "patterns";
  state.patternScope = JSON.stringify([run.organization_id, run.user_id]);
  state.patternThread = "";
  patternComparison.scope = "";
  patternComparison.thread = "";
  visibilityPatternGroup = null;
  element("pattern-type").value = "visibility";
  visibilityField("kind").value = kind;
  visibilityField("target").replaceChildren(new Option(target, target, true, true));
  visibilityField("token").value = String(token + 1);
  visibilityField("source").value = "";
  visibilityField("order").value = "recurrence";
  visibilityField("repeat").checked = false;
  for (const field of visibilityToolFields) visibilityField(field).value = "";
  for (const date of [...visibilityPatternDateFields, ...visibilityComparisonDateFields]) visibilityField(date).value = "";
  linkNotice();
  render();
  visibilityField("token").focus();
}

function writeVisibilityPatternLink(params) {
  const alignment = visibilityField("target").value ? "target" : "token";
  for (const field of ["kind", alignment, "source", "tool_name", ...visibilityPatternDateFields]) {
    if (visibilityField(field).value) params.set(field, visibilityField(field).value);
  }
  if (visibilityField("order").value === "magnitude") params.set("order", "magnitude");
  if (patternComparison.scope) {
    for (const field of ["tool_name_b", ...visibilityComparisonDateFields]) {
      if (visibilityField(field).value) params.set(field, visibilityField(field).value);
    }
  }
  if (visibilityField("repeat").checked) params.set("recurring", "1");
  if (visibilityPatternGroup) {
    params.set("group", visibilityPatternGroup);
    if (visibilityPatternUnit) params.set("timeline", visibilityPatternUnit);
    if (visibilityPatternPeriod) params.set("period", visibilityPatternPeriod);
  }
}

function restoreVisibilityPatternLink(params) {
  visibilityPatternGroup = params.get("group");
  const unit = params.get("timeline");
  visibilityPatternUnit = visibilityPatternGroup && visibilityTimeUnits.has(unit) ? unit : "";
  visibilityPatternPeriod = visibilityPatternUnit ? (params.get("period") ?? "") : "";
  visibilityField("kind").value = params.get("kind") === "pair" ? "pair" : "source";
  visibilityField("order").value = params.get("order") === "magnitude" ? "magnitude" : "recurrence";
  visibilityField("token").value = params.get("token") ?? "1";
  const target = params.get("target") ?? "";
  visibilityField("target").replaceChildren(new Option(target, target, true, true));
  visibilityField("repeat").checked = params.get("recurring") === "1";
  const source = params.get("source") ?? "";
  visibilityField("source").replaceChildren(new Option("All named sources", ""));
  if (source) visibilityField("source").add(new Option(source, source, true, true));
  for (const field of visibilityToolFields) {
    const tool = params.get(field) ?? "";
    visibilityField(field).replaceChildren(new Option(tool, tool, true, true));
  }
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

function visibilityToolFilter(field, records, thread) {
  const select = visibilityField(field);
  const selected = select.value;
  const tools = new Set(records.filter(({ occurrence }) => !thread || patternThreadKey(occurrence) === thread)
    .map(({ report }) => report.baseline_tool_call?.name).filter(Boolean));
  if (selected) tools.add(selected);
  select.replaceChildren(new Option("All decisions", ""), ...[...tools].sort().map((name) => new Option(name, name)));
  select.value = selected;
  visibilityField(field + "-field").hidden = !tools.size;
  return selected;
}

function renderVisibilityPatterns(list, scope, scopes) {
  const entries = visibilityReportsInScope(scope);
  updatePatternThreadFilter([{ occurrences: entries.map(({ occurrence }) => occurrence) }]);
  const secondScope = scopes.get(patternComparison.scope);
  const secondEntries = secondScope ? visibilityReportsInScope(secondScope) : [];
  const second = comparisonPatternFilter(scopes, [{ occurrences: secondEntries.map(({ occurrence }) => occurrence) }]);
  visibilityField("compare-dates").hidden = !second;
  const tool = visibilityToolFilter("tool_name", entries, state.patternThread);
  const secondTool = second && visibilityToolFilter("tool_name_b", secondEntries, second.thread);
  const tokenInput = visibilityField("token");
  const kind = visibilityField("kind").value;
  const select = visibilityField("source");
  const targetSelect = visibilityField("target");
  const selected = select.value;
  const target = targetSelect.value;
  const sources = new Set();
  const targets = new Set();
  for (const [records, thread, name] of [[entries, state.patternThread, tool], [secondEntries, second?.thread, secondTool]]) {
    for (const { report, call, occurrence } of records) {
      if (thread && patternThreadKey(occurrence) !== thread) continue;
      if (name && report.baseline_tool_call?.name !== name) continue;
      for (const item of report.targets || []) targets.add(item.key);
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
  if (target) targets.add(target);
  targetSelect.replaceChildren(new Option("Exact token and prefix", ""), ...[...targets].sort().map((key) => new Option(key, key)));
  targetSelect.value = target;
  visibilityField("target-field").hidden = !targets.size;
  visibilityField("token-field").hidden = Boolean(target);
  const panel = node("section", undefined, "panel visibility-pattern-results");
  list.append(panel);
  if (!target && (!tokenInput.validity.valid || !Number.isSafeInteger(tokenInput.valueAsNumber))) {
    panel.append(node("p", "Enter a positive whole number for the output token.", "muted"));
    return;
  }
  const primary = patternScopeSelection(scope, state.patternThread, visibilityPatternDateFields.map(visibilityField));
  const compare = second && patternScopeSelection(second.scope, second.thread, visibilityComparisonDateFields.map(visibilityField));
  if (!primary || (second && !compare)) {
    const side = second ? " for Scope " + (primary ? "B" : "A") : "";
    panel.append(node("p", "Enter complete dates" + side + ". The From date must precede the Before date.", "muted"));
    return;
  }
  const selection = target ? { target } : { output_token: tokenInput.valueAsNumber - 1 };
  const spec = { version: 1, kind, ...selection, ...primary, order: visibilityField("order").value, limit: 20 };
  if (tool) spec.tool_name = tool;
  if (compare) {
    spec.compare = compare;
    if (secondTool) spec.compare.tool_name = secondTool;
  }
  if (selected) spec.source_id = selected;
  if (visibilityField("repeat").checked) spec.min_conversations = 2;
  const captions = second ? [
    [patternScopeCaption(scope, element("pattern-thread")), tool].filter(Boolean).join(" · "),
    [patternScopeCaption(second.scope, element("pattern-compare-thread")), secondTool].filter(Boolean).join(" · "),
  ] : [];
  const results = node("div");
  const back = node("button", "All comparison groups");
  back.type = "button";
  const controls = node("div", undefined, "visibility-time-controls");
  const timePicker = node("select");
  for (const [value, label] of visibilityTimeUnits) timePicker.append(new Option(label, value));
  const timeLabel = node("label", "Group by UTC", "activation-picker");
  timeLabel.append(timePicker);
  const periods = node("button", "All periods");
  periods.type = "button";
  controls.append(timeLabel, periods);
  panel.append(back, controls, results);
  function load(group, offset = 0, focus = false, compareOffset = 0) {
    visibilityPatternGroup = group;
    back.hidden = !group;
    controls.hidden = !group;
    if (!group) {
      visibilityPatternUnit = "";
      visibilityPatternPeriod = "";
    }
    timePicker.value = visibilityPatternUnit;
    periods.hidden = !visibilityPatternPeriod;
    const request = { ...spec, offset, group };
    if (visibilityPatternUnit) {
      request.timeline = { unit: visibilityPatternUnit };
      if (visibilityPatternPeriod) request.timeline.period = visibilityPatternPeriod;
    }
    if (compare && group) request.compare_offset = compareOffset;
    showPatternPage(results, "/api/visibility-patterns", request, (page) => {
      const title = target ? "Effects on " + target : "Effects at token " + (spec.output_token + 1);
      const heading = node("h2", group ? visibilityGroupLabel(page.group) : title);
      heading.tabIndex = -1;
      results.append(heading);
      if (request.timeline?.period) {
        results.append(node("p", visibilityPeriodLabel(request.timeline.unit, request.timeline.period), "count"));
      }
      const timeline = page.kind === "visibility_pattern_timeline";
      if (timeline) {
        renderVisibilityTimeline(results, page, captions, (period) => {
          visibilityPatternPeriod = period;
          load(group, 0, true);
        });
      } else if (compare) {
        renderVisibilityComparison(results, page, captions, group, offset, compareOffset, load);
      } else if (group) {
        appendVisibilityModel(results, page.group.comparison);
        if (target && !request.timeline?.period) appendVisibilityCoverage(results, page.coverage);
        appendVisibilityGroupSummary(results, page.group, page.metric);
        renderVisibilityOccurrences(results, page.occurrences);
      } else {
        renderVisibilityGroups(results, page, (key) => load(key, 0, true));
      }
      if (timeline || !compare || !group) patternPager(results, offset, page.next_offset, (next) => load(group, next, true));
      if (focus && state.view === "patterns") heading.focus();
    });
  }
  timePicker.addEventListener("change", () => {
    visibilityPatternUnit = timePicker.value;
    visibilityPatternPeriod = "";
    load(visibilityPatternGroup);
  });
  periods.addEventListener("click", () => {
    visibilityPatternPeriod = "";
    load(visibilityPatternGroup, 0, true);
  });
  back.addEventListener("click", () => load(null, 0, true));
  load(visibilityPatternGroup);
}

function visibilityGroupLabel(group) {
  return group.comparison.sources.map(({ source_id, source_version }) =>
    [source_id, source_version].filter(Boolean).join(" · ")).join(" + ");
}

function appendVisibilityGroupSummary(parent, group, metric) {
  parent.append(node("p", visibilityEffectRange(group.effect) + " · " + visibilityEffectUnit(metric), "count"));
  parent.append(node("p", [countLabel(group.conversation_count, "conversation"), countLabel(group.user_count, "user"),
    countLabel(group.organization_count, "organization"), countLabel(group.decision_count, "decision")].join(" · "), "muted"));
  if (group.output_prefix_count !== undefined) {
    const count = group.output_prefix_count;
    parent.append(node("p", count + (count === 1 ? " recorded prefix" : " recorded prefixes"), "muted"));
  }
}

function visibilityEffectRange({ minimum, maximum }) {
  const number = visibilityEffectFormat.format;
  return minimum === maximum ? number(minimum) : number(minimum) + " to " + number(maximum);
}

function visibilityEffectUnit(metric) {
  return metric === "interaction_log_units" ? "interaction log units" : "change in log probability";
}

function visibilityPeriodLabel(unit, period) {
  if (period === "undated") return "Time unavailable";
  if (unit === "week") return "Week of " + period;
  return unit === "month" ? period.slice(0, -3) : period;
}

function renderVisibilityTimeline(parent, page, captions, openPeriod) {
  appendVisibilityModel(parent, page.group.comparison);
  appendVisibilityCoverage(parent, page.coverage_a, captions.length ? " in Scope A" : "");
  if (captions.length) appendVisibilityCoverage(parent, page.coverage_b, " in Scope B");
  const unit = visibilityEffectUnit(page.metric);
  parent.append(node("p", "Values show " + unit + ". All periods share a scale centered on zero.", "muted"));
  const table = node("table", undefined, "visibility-time-table");
  const headings = table.createTHead().insertRow();
  const period = node("th", visibilityTimeUnits.get(page.unit));
  period.scope = "col";
  headings.append(period);
  for (const [index, caption] of (captions.length ? captions : [""]).entries()) {
    const label = node("th", captions.length ? (index ? "Scope B" : "Scope A") : "Measured effect");
    label.scope = "col";
    if (caption) label.append(node("small", caption, "muted"));
    headings.append(label);
  }
  const body = table.createTBody();
  const scale = page.maximum_absolute_effect || 1;
  for (const row of page.rows) {
    const line = body.insertRow();
    const date = node("th");
    date.scope = "row";
    const button = node("button", visibilityPeriodLabel(page.unit, row.period));
    button.type = "button";
    button.addEventListener("click", () => openPeriod(row.period));
    date.append(button);
    line.append(date);
    for (const scope of (captions.length ? [row.scope_a, row.scope_b] : [row.scope_a])) {
      const cell = line.insertCell();
      if (!scope) {
        cell.append(node("span", "No measurements", "muted"));
        continue;
      }
      const track = node("span", undefined, "visibility-time-track");
      track.setAttribute("aria-hidden", "true");
      const mark = node("span", undefined, "visibility-time-mark");
      const { minimum, maximum } = scope.effect;
      mark.style.left = (0.5 + minimum / scale / 2) * 100 + "%";
      mark.style.width = (maximum / scale - minimum / scale) * 50 + "%";
      track.append(mark);
      cell.append(node("strong", visibilityEffectRange(scope.effect)), track,
        node("small", countLabel(scope.conversation_count, "conversation") + " · " + countLabel(scope.decision_count, "decision"), "muted"));
    }
  }
  if (!page.rows.length) parent.append(node("p", "There are no periods on this page.", "muted"));
  else parent.append(table);
}

function appendVisibilityModel(parent, comparison) {
  const output = comparison.target || "Prefix " + comparison.output_prefix_sha256.slice(0, 8);
  const details = [comparison.model_id, comparison.runtime, output];
  if (comparison.decision_kind === "tool_call") details.push("Tool proposal");
  const model = node("small", details.join(" · "), "anchor");
  model.title = "Model revision " + comparison.model_revision;
  parent.append(model);
  for (const source of comparison.sources) {
    if (source.boundary_whitespace) parent.append(visibilityBoundaryNote(source.boundary_whitespace, source.source_id));
  }
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
  if (coverage.reports_with_target !== undefined) {
    parent.append(node("p", "Target declared in " + coverage.reports_with_target + " of "
      + countLabel(coverage.reports_in_scope, "visibility report") + scope + ".", "muted"));
  }
  const missing = coverage.measurements_without_source_id;
  if (missing) parent.append(node("p", countLabel(missing, "measurement") + scope
    + " could not be grouped because a source ID was unavailable.", "muted"));
}

function renderVisibilityComparison(parent, page, captions, key, offset, compareOffset, load) {
  if (!visibilityPatternPeriod) {
    appendVisibilityCoverage(parent, page.coverage_a, " in Scope A");
    appendVisibilityCoverage(parent, page.coverage_b, " in Scope B");
  }
  if (key) {
    appendVisibilityModel(parent, page.group.comparison);
    parent.append(patternScopeColumns(captions, [page.scope_a, page.scope_b], (column, evidence, index) => {
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
    line.append(patternScopeColumns(captions, [group.scope_a, group.scope_b], (column, summary) => {
      appendVisibilityGroupSummary(column, summary, page.metric);
    }));
    parent.append(line);
  }
}

function renderVisibilityOccurrences(parent, occurrences) {
  for (const row of occurrences) {
    const line = node("article", undefined, "visibility-pattern-row");
    const label = row.conversation_id || row.location.run_id;
    line.append(node("h3", label));
    const recorded = [row.organization_id, row.user_id, row.turn_id, row.tool_name && "Proposed " + row.tool_name].filter(Boolean);
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
    const first = row.output_token + 1;
    const tokens = row.output_token_end > first ? "tokens " + first + "–" + row.output_token_end : "token " + first;
    links.append(patternEvidenceLink(row.inspector_path, "Open " + tokens));
    for (const branch of row.mask_continuations || []) {
      links.append(patternEvidenceLink(branch.inspector_path, "Open masked continuation"));
    }
    row.source_inspector_paths.forEach((path, index) => links.append(patternEvidenceLink(path, row.context_ids[index])));
    line.append(node("strong", visibilityEffectFormat.format(row.effect)), links);
    if (row.output_prefix_sha256) line.append(node("small", "Prefix " + row.output_prefix_sha256.slice(0, 8), "anchor"));
    parent.append(line);
  }
}
