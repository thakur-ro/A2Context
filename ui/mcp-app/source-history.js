import { button, countLabel, element, scopeColumns } from "./dom.js";

const tool = "query_context_sources";
const signed = new Intl.NumberFormat(undefined, { maximumFractionDigits: 5, signDisplay: "exceptZero" });
const filters = [
  ["all", "All occurrences"],
  ["conflicts", "Rule conflicts"],
  ["measured", "Measured effects"],
  ["unmeasured", "Without measurements"],
  ["activation_swaps", "Decoder-block source swaps"],
];

export function sourceHistoryTitle(data, args) {
  switch (data.kind) {
    case "context_source_profiles":
    case "context_source_profile_comparison":
      return "Context sources";
    case "context_source_occurrences":
    case "context_source_occurrence_comparison":
      return data.source_id || data.source?.source_id || args.source_id || "Source history";
  }
}

export function sourceHistoryBackLabel(kind) {
  if (kind.startsWith("context_source_occurrence")) return "Back to source history";
  if (kind.startsWith("context_source_profile")) return "Back to context sources";
}

function appendSummary(parent, profile) {
  const coverage = [
    countLabel(profile.occurrence_count, "occurrence"),
    countLabel(profile.call_count, "call"),
    countLabel(profile.conversation_count, "identified conversation"),
    profile.measured_occurrences + " measured",
  ];
  if (profile.conflicted_occurrences) coverage.push(profile.conflicted_occurrences + " with a rule conflict");
  parent.append(element("p", coverage.join(" · ")));
  const effects = [
    ["activation_swap_occurrences", "with decoder-block source swaps"],
    ["masked_occurrences", "with source masks"],
    ["replayed_occurrences", "with measured replay"],
    ["candidate_probed_occurrences", "with candidate scores"],
    ["shadow_probed_occurrences", "with shadow comparisons"],
    ["shadow_unavailable_occurrences", "with unavailable shadow comparisons"],
  ].filter(([key]) => profile[key]).map(([key, label]) => profile[key] + " " + label);
  if (effects.length) parent.append(element("p", effects.join(" · "), "muted"));
  const versions = element("ul", undefined, "source-versions");
  for (const version of profile.versions) {
    const info = [
      version.source_version ?? "Version unavailable",
      version.included_occurrences + " included",
      version.unavailable_occurrences + " unavailable",
    ];
    if (version.captured_forms > 1) info.push(version.captured_forms + " captured forms");
    versions.append(element("li", info.join(" · ")));
  }
  parent.append(versions);
}

function controls(parent, page, requestPage, detail) {
  const fields = element("div", undefined, "actions source-filters");
  const label = element("label", "Evidence ");
  const filter = element("select");
  filter.setAttribute("aria-label", "Evidence");
  for (const [value, text] of filters) filter.add(new Option(text, value));
  filter.value = page.args.evidence_filter ?? "all";
  label.append(filter);
  fields.append(label);
  let search;
  if (!detail) {
    search = element("input");
    search.type = "search";
    search.setAttribute("aria-label", "Search sources");
    search.value = page.args.search ?? "";
    const field = element("label", "Search sources ");
    field.append(search);
    fields.append(field);
  }
  const apply = button("Apply filters", () => {
    const args = { ...page.args, evidence_filter: filter.value, offset: 0, compare_offset: 0 };
    if (search) args.search = search.value.trim();
    requestPage(tool, args, page.title, page.back);
  });
  search?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") apply.click();
  });
  fields.append(apply);
  parent.append(fields);
}

function appendOccurrence(parent, item, page, requestPage) {
  const row = element("section", undefined, "row");
  const conversation = item.conversation_id || item.run_id;
  const location = { run_id: item.run_id, call_id: item.call_id };
  if (item.linked_decision_ids.length === 1) location.decision_id = item.linked_decision_ids[0];
  const open = (label, selected) => button(label, () => requestPage(
    "get_turn_evidence", selected, conversation, { page, previous: page.back },
  ));
  row.append(element("h3", conversation),
    element("p", [item.started_at || "Date unavailable", item.turn_id, item.call_id].filter(Boolean).join(" · "), "muted"),
    element("p", [item.organization_id, item.user_id, item.source_version, item.availability].filter(Boolean).join(" · "), "muted"),
    open("Open turn evidence", location));
  const paths = element("details", undefined, "source-paths");
  paths.append(element("summary", "Local inspector paths"), element("p", "Captured source"), element("code", item.inspector_path));
  const swap = item.activation_swap_example;
  const position = swap?.position;
  const prediction = position?.output_token === undefined ? "Prediction position " + position?.prediction_position : "Token " + (position.output_token + 1);
  const swapLabel = swap && [position.label, prediction, "block " + swap.block,
    signed.format(swap.change_log_odds) + " log odds"].filter(Boolean).join(" · ");
  const mask = item.mask_example;
  const replay = item.replay_example;
  const candidate = item.candidate_example;
  const shadow = item.shadow_example;
  for (const [label, evidence, summary] of [
    ["Largest source swap", swap, swapLabel],
    ["Largest source mask", mask, mask && "Token " + (mask.output_token + 1) + " · "
      + signed.format(mask.change_in_log_probability) + " change in log probability"],
    ["Replay", replay, replay && signed.format(replay.difference_pp) + " percentage points"],
    ["Candidate probe", candidate, candidate && signed.format(candidate.change_log_odds) + " log odds"],
    ["Shadow probe", shadow, shadow && (shadow.difference_pp == null ? "Effect unavailable" : signed.format(shadow.difference_pp) + " percentage points")],
    ["Preflight conflict", item.preflight_conflict_example, item.preflight_conflict_example?.key],
    ["Decision conflict", item.decision_conflict_example, item.decision_conflict_example?.key],
  ]) {
    if (!evidence) continue;
    row.append(element("p", label + " · " + summary));
    const decision = evidence.location ?? (evidence.decision_id ? { ...location, decision_id: evidence.decision_id } : null);
    if (decision) row.append(open("Open " + label.toLowerCase() + " decision", decision));
    paths.append(element("p", label), element("code", evidence.inspector_path));
  }
  row.append(paths);
  parent.append(row);
}

export function renderSourceHistory(parent, page, requestPage, pageControls) {
  const { data, args, back } = page;
  const detail = data.kind.startsWith("context_source_occurrence");
  const compared = data.kind.endsWith("_comparison");
  const canQuery = args.version === 1;
  if (canQuery) controls(parent, page, requestPage, detail);
  if (!detail) {
    parent.append(element("p", countLabel(data.total_sources, "source"), "muted"));
    for (const profile of data.rows) {
      const row = element("section", undefined, "row");
      row.append(canQuery ? button(profile.source_id, () => requestPage(
        tool, { ...args, source_id: profile.source_id, offset: 0, compare_offset: 0 },
        profile.source_id, { page, previous: back },
      )) : element("h2", profile.source_id));
      if (compared) scopeColumns(row, args, [profile.scope_a, profile.scope_b], appendSummary, "No recorded occurrences");
      else appendSummary(row, profile);
      parent.append(row);
    }
    if (canQuery) pageControls(page, tool, back);
    return;
  }
  const show = (section, side, index) => {
    if (!side.source) {
      section.append(element("p", "No recorded occurrences", "muted"));
      return;
    }
    appendSummary(section, side.source);
    for (const item of side.rows) appendOccurrence(section, item, page, requestPage);
    if (canQuery) pageControls(page, tool, back, {
      container: section, offsetKey: index ? "compare_offset" : "offset", next: side.next_offset,
    });
  };
  if (compared) scopeColumns(parent, args, [data.scope_a, data.scope_b], show, "No recorded occurrences");
  else show(parent, data, 0);
}
