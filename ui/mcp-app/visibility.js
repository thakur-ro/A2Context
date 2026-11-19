import { button, countLabel, element } from "./dom.js";

const maskOutcomeLabels = {
  changed_tool: "Tool changed", changed_arguments: "Arguments changed", same_proposal: "Same proposal",
  unparsed_proposal: "No parsed proposal", changed_output: "Output changed", same_output: "Same output",
  token_limit: "Token limit reached", mixed: "Mixed outcomes",
};

const patternTool = "query_visibility_patterns";
const measurementTool = "query_source_visibility";
const format = new Intl.NumberFormat(undefined, { maximumFractionDigits: 5, signDisplay: "exceptZero" });
const units = [["", "All occurrences"], ["day", "Day"], ["week", "Week"], ["month", "Month"]];

function groupTitle(group) {
  return group.comparison.sources.map(({ source_id, source_version }) =>
    [source_id, source_version].filter(Boolean).join(" · ")).join(" + ");
}

export function visibilityTitle(data) {
  switch (data.kind) {
    case "visibility_pattern_groups":
    case "visibility_pattern_comparison":
      return "Masked-source effects";
    case "visibility_pattern_evidence":
    case "visibility_pattern_comparison_evidence":
    case "visibility_pattern_timeline":
      return groupTitle(data.group);
    case "visibility_sources":
    case "visibility_pairs":
      return "Output token " + (data.token.output_token + 1);
    case "visibility_continuations":
      return "Decoding with sources masked";
  }
}

export function visibilityBackLabel(kind) {
  switch (kind) {
    case "visibility_pattern_groups":
    case "visibility_pattern_comparison":
      return "Back to pattern groups";
    case "visibility_pattern_evidence":
    case "visibility_pattern_comparison_evidence":
      return "Back to contributing turns";
    case "visibility_pattern_timeline":
      return "Back to periods";
    case "visibility_sources":
    case "visibility_pairs":
      return "Back to measured token";
    case "visibility_continuations":
      return "Back to masked continuations";
  }
}

function scopeName({ scope, tool_name } = {}) {
  const identity = [scope?.organization_id, scope?.user_id].filter(Boolean).join(" / ") || "Workspace";
  return [identity, tool_name].filter(Boolean).join(" · ");
}

function sectionSides(parent, args, [left, right], render, empty = "No measurements") {
  if (right === undefined) {
    render(parent, left, 0);
    return;
  }
  const columns = element("div", undefined, "visibility-columns");
  for (const [index, side] of [left, right].entries()) {
    const section = element("section");
    section.append(element("h3", index ? "Scope B" : "Scope A"));
    if (args.version === 1) section.append(element("p", scopeName(index ? args.compare : args), "muted"));
    if (side) render(section, side, index);
    else section.append(element("p", empty, "muted"));
    columns.append(section);
  }
  parent.append(columns);
}

function summary(parent, value, metric, scale) {
  const { minimum, maximum } = value.effect;
  const range = minimum === maximum ? format.format(minimum) : format.format(minimum) + " to " + format.format(maximum);
  const unit = metric === "interaction_log_units" ? "interaction log units" : "change in log probability";
  parent.append(element("p", range + " · " + unit));
  if (scale !== undefined) {
    const extent = scale || 1;
    const track = element("span", undefined, "visibility-range");
    track.setAttribute("aria-hidden", "true");
    const mark = element("span");
    mark.style.left = (0.5 + minimum / extent / 2) * 100 + "%";
    mark.style.width = (maximum / extent - minimum / extent) * 50 + "%";
    track.append(mark);
    parent.append(track);
  }
  const counts = [countLabel(value.conversation_count, "conversation"), countLabel(value.decision_count, "decision")];
  if (value.output_prefix_count !== undefined) {
    counts.push(value.output_prefix_count + (value.output_prefix_count === 1 ? " recorded prefix" : " recorded prefixes"));
  }
  parent.append(element("p", counts.join(" · "), "muted"));
  const replay = value.continuation_outcomes;
  if (replay) {
    const outcomes = Object.entries(replay.by_outcome).map(([kind, count]) => maskOutcomeLabels[kind] + " " + count);
    parent.append(element("p", ["Replayed " + replay.decisions_tested + " of " + countLabel(value.decision_count, "decision"),
      ...outcomes].join(" · "), "muted"));
  }
}

function boundaryNote({ before, after }, label) {
  const parts = [before && before.length + " before", after && after.length + " after"].filter(Boolean);
  const prefix = label ? label + " · " : "";
  const note = element("p", prefix + "Extra whitespace bytes in mask · " + parts.join(" and "), "muted");
  note.title = JSON.stringify({ before, after });
  return note;
}

function provenance(parent, { model_id, runtime, output_token, target, sources }) {
  parent.append(element("p", [target || "Token " + (output_token + 1), model_id, runtime].join(" · "), "muted"));
  for (const source of sources) {
    if (source.boundary_whitespace) parent.append(boundaryNote(source.boundary_whitespace, source.source_id));
  }
}

function coverage(parent, data) {
  for (const [entry, suffix] of [[data.coverage, ""], [data.coverage_a, data.coverage_b ? " in Scope A" : ""], [data.coverage_b, " in Scope B"]]) {
    if (entry?.reports_with_target !== undefined) {
      parent.append(element("p", "Target declared in " + entry.reports_with_target + " of "
        + countLabel(entry.reports_in_scope, "visibility report") + suffix + ".", "muted"));
    }
  }
}

function periodLabel(unit, period) {
  if (period === "undated") return "Time unavailable";
  if (unit === "week") return "Week of " + period;
  return unit === "month" ? period.slice(0, -3) : period;
}

export function renderVisibility(parent, page, requestPage, pageControls) {
  const { args, data, back } = page;
  const drill = { page, previous: back };
  if (page.kind === "visibility_continuations") {
    renderMaskContinuations(parent, page, requestPage, pageControls);
    return;
  }
  if (page.kind === "visibility_sources" || page.kind === "visibility_pairs") {
    renderMeasurement(parent, page, requestPage, drill);
    return;
  }
  const navigation = args.version === 1;
  const compared = data.coverage_b != null;
  const query = (updates, title, previous) => {
    const next = { ...args, offset: 0, compare_offset: 0, ...updates };
    if (next.timeline === undefined) delete next.timeline;
    requestPage(patternTool, next, title, previous);
  };
  if (args.tool_name && !compared) parent.append(element("p", "Recorded tool · " + args.tool_name, "muted"));
  if (!args.timeline?.period) coverage(parent, data);
  if (data.groups) {
    if (navigation) {
      const label = element("label", "Order by ", "visibility-controls");
      const select = element("select");
      select.append(new Option("Most conversations", "recurrence"), new Option("Largest measured effect", "magnitude"));
      select.value = data.order ?? "recurrence";
      select.onchange = () => query({ order: select.value }, page.title, back);
      label.append(select);
      parent.append(label);
    }
    for (const group of data.groups) {
      const row = element("section", undefined, "row");
      const title = groupTitle(group);
      row.append(navigation ? button(title, () => query({ group: group.key }, title, drill)) : element("h2", title));
      provenance(row, group.comparison);
      sectionSides(row, args, [compared ? group.scope_a : group, group.scope_b], (column, value) => summary(column, value, data.metric));
      parent.append(row);
    }
    if (!data.groups.length) parent.append(element("p", "No comparable measurements match this view.", "muted"));
    if (navigation) pageControls(page, patternTool, back);
    return;
  }
  provenance(parent, data.group.comparison);
  if (navigation && !args.timeline?.period) {
    const label = element("label", "Group by UTC ", "visibility-controls");
    const select = element("select");
    for (const [value, title] of units) select.append(new Option(title, value));
    select.value = args.timeline?.unit || "";
    select.onchange = () => query({ timeline: select.value ? { unit: select.value } : undefined }, page.title, back);
    label.append(select);
    parent.append(label);
  }
  if (page.kind === "visibility_pattern_timeline") {
    parent.append(element("p", "All periods share a scale centered on zero.", "muted"));
    for (const period of data.rows) {
      const row = element("section", undefined, "row");
      const title = periodLabel(data.unit, period.period);
      row.append(navigation ? button(title, () => query(
        { timeline: { unit: data.unit, period: period.period } }, title, drill,
      )) : element("h2", title));
      sectionSides(row, args, [period.scope_a, compared ? period.scope_b : undefined],
        (column, value) => summary(column, value, data.metric, data.maximum_absolute_effect));
      parent.append(row);
    }
    if (!data.rows.length) parent.append(element("p", "There are no periods on this page.", "muted"));
    if (navigation) pageControls(page, patternTool, back);
    return;
  }
  sectionSides(parent, args, [compared ? data.scope_a : data, data.scope_b], (column, scope, index) => {
    summary(column, scope.group, data.metric);
    for (const item of scope.occurrences) renderOccurrence(column, item, data.metric, requestPage, drill);
    if (navigation) pageControls(page, patternTool, back, {
      container: column, offsetKey: index ? "compare_offset" : "offset", next: scope.next_offset,
    });
  });
}

function renderOccurrence(parent, item, metric, requestPage, back) {
  const row = element("section", undefined, "row");
  const first = item.output_token + 1;
  const span = item.output_token_end > first ? "Tokens " + first + "–" + item.output_token_end : "Token " + first;
  row.append(element("h3", item.conversation_id || item.location.run_id));
  const labels = [item.organization_id, item.user_id, item.tool_name && "Proposed " + item.tool_name,
    item.started_at || "Time unavailable"];
  row.append(element("p", labels.filter(Boolean).join(" · "), "muted"));
  row.append(element("p", span + " · " + format.format(item.effect)));
  const view = { output_token: item.output_token, kind: metric === "interaction_log_units" ? "pairs" : "sources" };
  if (view.kind === "pairs") view.context_ids = item.context_ids;
  else view.context_id = item.context_ids[0];
  row.append(button("Inspect token " + first, () => requestPage(
    measurementTool, { version: 1, location: item.location, report_id: item.report_id, view },
    "Output token " + first, back,
  )));
  for (const branch of item.mask_continuations || []) {
    row.append(button("Open masked continuation", () => requestPage(measurementTool, {
      version: 1, location: item.location, report_id: item.report_id,
      view: { kind: "continuations", output_token: branch.start_output_token, context_ids: branch.context_ids },
    }, "Decoding with sources masked", back)));
  }
  parent.append(row);
}

function renderMaskContinuations(parent, page, requestPage, pageControls) {
  const { data, args, back, title } = page;
  if (data.captured_excerpts_enabled && !args.include_excerpts) {
    parent.append(button("Show generated text", () => requestPage(measurementTool, { ...args, include_excerpts: true }, title, back)));
  }
  for (const value of data.rows) {
    const row = element("section", undefined, "row");
    const changed = value.first_changed_output_token;
    row.append(element("h2", value.context_ids.join(" + ")));
    row.append(element("p", "Decoding resumes at token " + (value.start_output_token + 1), "muted"));
    row.append(element("p", maskOutcomeLabels[value.outcome] + (changed == null ? "" : " · First change at token " + (changed + 1))));
    if (value.text_excerpt) row.append(element("pre", value.text_excerpt.text + (value.text_excerpt.truncated ? "…" : "")));
    const paths = element("details");
    paths.append(element("summary", "Local inspector paths"), element("p", value.inspector_path, "inspector-path"));
    for (const path of value.source_inspector_paths) paths.append(element("p", path, "inspector-path"));
    row.append(paths);
    parent.append(row);
  }
  if (!data.rows.length) parent.append(element("p", "No saved continuations match this selection.", "muted"));
  if (args.version === 1) pageControls(page, measurementTool, back);
}

function renderMeasurement(parent, page, requestPage, drill) {
  const { data, args, back, title } = page;
  parent.append(element("p", "Original token log probability " + format.format(data.token.baseline_log_probability), "muted"));
  if (data.token.fragment) parent.append(element("p", data.token.fragment.text + (data.token.fragment.truncated ? "…" : "")));
  if (data.captured_excerpts_enabled && !args.include_excerpts) {
    parent.append(button("Show token text", () => requestPage(measurementTool, { ...args, include_excerpts: true }, title, back)));
  }
  const paths = element("details");
  paths.append(element("summary", "Local inspector paths"));
  for (const value of data.rows) {
    const row = element("section", undefined, "row");
    if (data.kind === "visibility_pairs") {
      row.append(element("h2", value.context_ids.join(" + ")));
      for (const [label, change] of [
        ["Only " + value.context_ids[0], value.only_first.change_in_log_probability],
        ["Only " + value.context_ids[1], value.only_second.change_in_log_probability],
        ["Both masked", value.both_masked.change_in_log_probability],
        ["Interaction", value.interaction_log_units],
      ]) row.append(element("p", label + " · " + format.format(change)));
      for (const path of [value.inspector_path, ...value.source_inspector_paths]) paths.append(element("p", path, "inspector-path"));
    } else {
      row.append(element("h2", value.context_id), element("p", value.effect
        ? format.format(value.effect.change_in_log_probability) + " change in log probability"
        : value.unavailable.replaceAll("_", " ")));
      if (value.boundary_whitespace) row.append(boundaryNote(value.boundary_whitespace));
      if (value.inspector_path) paths.append(element("p", value.inspector_path, "inspector-path"));
      paths.append(element("p", value.source_inspector_path, "inspector-path"));
    }
    parent.append(row);
  }
  if (!data.rows.length) parent.append(element("p", "No measurements match this selection.", "muted"));
  if (data.kind === "visibility_sources" && !data.rows.some(({ inspector_path }) => inspector_path)) {
    paths.append(element("p", data.token.inspector_path, "inspector-path"));
  }
  const actions = element("div", undefined, "actions");
  if (data.report.measured_continuations) actions.append(button("View saved continuations", () => requestPage(
    measurementTool, { ...args, offset: 0, view: { kind: "continuations" } }, "Decoding with sources masked", drill,
  )));
  actions.append(button("Open turn evidence", () => requestPage("get_turn_evidence", data.location, "Turn evidence", drill)));
  parent.append(paths, actions);
}
