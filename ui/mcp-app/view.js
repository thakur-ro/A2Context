import { App } from "@modelcontextprotocol/ext-apps";
import { button, countLabel, element, scopeColumns } from "./dom.js";
import { renderSourceHistory, sourceHistoryBackLabel, sourceHistoryTitle } from "./source-history.js";
import { renderVisibility, visibilityBackLabel, visibilityTitle } from "./visibility.js";

const app = new App({ name: "NexusWeave evidence", version: "1.0.0" });
const result = document.getElementById("result");
let initialQuery;
let current;
let generation = 0;

function showStatus(message, error = false) {
  const status = element("p", message, "muted");
  status.setAttribute("role", error ? "alert" : "status");
  result.replaceChildren(element("h1", "NexusWeave evidence"), status);
}

function payload(response) {
  const text = response.content?.find(({ type }) => type === "text")?.text;
  if (response.isError) throw new Error(text || "The evidence request failed.");
  return response.structuredContent ?? JSON.parse(text);
}

function openPage(page, back, focus = false) {
  current = page;
  page.back = back;
  result.replaceChildren();
  if (back) {
    const screenBack = back.page.kind.startsWith("screening_")
      ? (back.page.kind.endsWith("_groups") ? "Back to screened pairs" : "Back to contributing turns") : null;
    const replayBack = back.page.kind.startsWith("replay_effect_")
      ? (back.page.kind.endsWith("_groups") ? "Back to replay groups" : "Back to contributing replays") : null;
    const label = screenBack ?? replayBack ?? sourceHistoryBackLabel(back.page.kind) ?? visibilityBackLabel(back.page.kind) ?? (back.page.kind === "turn" ? "Back to turn evidence" :
      back.page.kind.startsWith("activation_evidence") ? "Back to activation evidence" :
      page.kind === "turn" ? "Back to contributing decisions" :
      page.kind.startsWith("activation_evidence") ? "Back to activation groups" : "Back to conflict groups");
    const control = button(label, () => {
      openPage(back.page, back.previous, true);
    });
    control.className = "back";
    result.append(control);
  }
  const heading = element("h1", page.title);
  result.append(heading);
  if (page.kind === "groups") renderGroups(page, back);
  else if (page.kind === "evidence") renderEvidence(page, back);
  else if (page.kind === "activation_groups" || page.kind === "activation_group_comparison") renderActivationGroups(page, back);
  else if (page.kind.startsWith("activation_evidence")) renderActivationEvidence(page, back);
  else if (/^(candidate|shadow|screening)_pattern_/.test(page.kind)) renderAssessmentPatterns(page, back);
  else if (page.kind.startsWith("replay_effect_")) renderReplayEffects(page, back);
  else if (page.kind.startsWith("visibility_")) renderVisibility(result, page, requestPage, pageControls);
  else if (page.kind.startsWith("context_source_")) renderSourceHistory(result, page, requestPage, pageControls);
  else if (page.kind === "pending_evidence") renderPending(page);
  else renderTurn(page);
  if (focus) {
    heading.tabIndex = -1;
    heading.focus();
  }
}

async function requestPage(tool, args, title, back) {
  const previous = current;
  const request = ++generation;
  showStatus("Loading evidence…");
  try {
    const data = payload(await app.callServerTool({ name: tool, arguments: args }));
    if (generation !== request) return;
    const kind = tool === "get_turn_evidence" ? "turn" : data.kind;
    openPage({ kind, title, args, data }, back, true);
  } catch (error) {
    if (generation !== request) return;
    showStatus(error.message, true);
    if (previous) result.append(button("Return to results", () => openPage(previous, previous.back, true)));
  }
}

function groupName(key, dimensions) {
  return key.map((values, index) => {
    if (dimensions[index] === "source_version") return [values[0] || "Unnamed source", values[1] || "Version unavailable"].join(" · ");
    if (dimensions[index] === "conversation") {
      const [source, organization, user, conversation] = values;
      return [conversation || "Conversation ID unavailable", organization, user, source].filter(Boolean).join(" · ");
    }
    if (dimensions[index] === "user") return [values[1] || "User ID unavailable", values[0]].filter(Boolean).join(" · ");
    return values.filter(Boolean).join(" · ") || "Unavailable";
  }).join(" → ");
}

function pageControls(page, tool, back, { container = result, offsetKey = "offset", next = page.data.next_offset } = {}) {
  const offset = page.args[offsetKey] || 0;
  const limit = page.args.limit || 20;
  if (!offset && next == null) return;
  const actions = element("div", undefined, "actions");
  for (const [label, target] of [["Previous page", offset ? Math.max(0, offset - limit) : null], ["Next page", next]]) {
    if (target != null) actions.append(button(label, () => requestPage(tool, { ...page.args, [offsetKey]: target }, page.title, back)));
  }
  container.append(actions);
}

function renderGroups(page, back) {
  const { totals, rows } = page.data;
  const unit = page.data.phase === "preflight" ? "checks" : "decisions";
  result.append(element("p", totals["conflicting_" + unit] + " conflicting " + unit + " · "
    + totals["comparable_" + unit] + " comparable · " + totals["scoped_" + unit] + " scoped", "muted"));
  for (const item of rows) {
    const { key, affected_conversations } = item;
    const conflicts = item["conflicting_" + unit];
    const name = groupName(key, page.args.group_by || []);
    const row = element("div", undefined, "row");
    if (conflicts && page.args.group_by) {
      row.append(button(name, () => requestPage("query_conflicts", { ...page.args, group: key, offset: 0 }, name, { page, previous: back })));
    } else {
      row.append(element("h2", name));
    }
    row.append(element("p", countLabel(conflicts, "conflict") + " · "
      + countLabel(affected_conversations, "identified conversation"), "muted"));
    result.append(row);
  }
  if (!rows.length) result.append(element("p", "No scoped " + unit + " match this view.", "muted"));
  if (page.args.group_by) pageControls(page, "query_conflicts", back);
}

function renderEvidence(page, back) {
  const groups = new Map();
  for (const row of page.data.rows) {
    const key = JSON.stringify(row.conversation_id
      ? [row.trace_source, row.organization_id, row.user_id, row.conversation_id] : [row.run_id]);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(row);
  }
  for (const rows of groups.values()) {
    const first = rows[0];
    const section = element("section", undefined, "row");
    section.append(element("h2", first.conversation_id || first.run_id));
    const owner = [first.user_id, first.organization_id, first.conversation_id ? null : "Conversation ID unavailable"].filter(Boolean).join(" · ");
    if (owner) section.append(element("p", owner, "muted"));
    for (const { run_id, call_id, decision_id, decision_label, rule_scope, turn_id } of rows) {
      const title = decision_label || rule_scope.join(" · ");
      const label = [title, turn_id, call_id].filter(Boolean).join(" · ");
      const control = button(label, () => requestPage(
        "get_turn_evidence", { run_id, call_id, decision_id }, title, { page, previous: back },
      ));
      const entry = element("div");
      entry.append(control);
      section.append(entry);
    }
    result.append(section);
  }
  if (!groups.size) result.append(element("p", page.data.phase === "preflight"
    ? "No conflicting checks in this group." : "No conflicting decisions in this group.", "muted"));
  if (page.args.group_by) pageControls(page, "query_conflicts", back);
}

function turnLocation({ run_id, call_id, decision }) {
  return { run_id, call_id, decision_id: decision?.id };
}

function probeGroupName({ source_id, source_version, candidate_ids, expected_kind }) {
  return [source_id, source_version, candidate_ids?.join(" vs ") ?? expected_kind?.replaceAll("_", " ")].filter(Boolean).join(" · ");
}

function appendProbeStats(parent, stats, label, shadow) {
  if (label) parent.append(element("h3", label));
  if (!stats) {
    parent.append(element("p", "No comparable measurements", "muted"));
    return;
  }
  const min = shadow ? stats.min_difference_pp : stats.min_change_log_odds;
  const max = shadow ? stats.max_difference_pp : stats.max_change_log_odds;
  const range = min == null ? "No measured shift" :
    (min === max ? signed.format(min) : signed.format(min) + " to " + signed.format(max)) + (shadow ? " pp" : " log odds");
  const counts = shadow
    ? [range, stats.increased && stats.increased + " increased", stats.decreased && stats.decreased + " decreased",
      stats.unchanged && stats.unchanged + " unchanged", stats.unavailable && stats.unavailable + " unavailable"].filter(Boolean)
    : [stats.reversals + " of " + stats.measurements + " edits reversed preference", range];
  const conversations = shadow ? countLabel(stats.measured_conversations, "conversation") + " with scored comparisons" :
    countLabel(stats.conversations, "identified conversation");
  parent.append(element("p", counts.join(" · ")), element("p",
    [countLabel(stats.calls, "call"), conversations, countLabel(stats.organizations, "organization")].join(" · "), "muted"));
  const targets = shadow && stats.edited_target_conversations;
  if (targets) parent.append(element("p", "Edited target by conversation · " +
    [targets.met + " met", targets.missed + " missed", targets.mixed + " mixed", targets.incomplete + " incomplete"].join(" · ")));
}

function screeningGroupName(group) {
  const relation = group.suggestions.map((value) => value.replaceAll("_", " ")).join(" / ");
  const sources = group.sources.map((source) => [source.source_id || source.role || source.kind,
    source.source_version].filter(Boolean).join(" · "));
  return relation + " · " + sources.join(" and ");
}

function appendScreeningStats(parent, stats, label) {
  if (label) parent.append(element("h3", label));
  if (!stats) {
    parent.append(element("p", "No comparable screened pairs", "muted"));
    return;
  }
  const counts = [countLabel(stats.flagged_conversations, "identified conversation") + " with flags"];
  if (stats.flagged_organizations > 1) counts.push(countLabel(stats.flagged_organizations, "organization"));
  if (stats.unscored_calls) counts.push(countLabel(stats.unscored_calls, "call") + " without scores");
  parent.append(element("p", stats.flagged_calls + " of " + stats.assessed_calls + " assessed calls flagged"),
    element("p", counts.join(" · "), "muted"));
  if (stats.flagged_calls) {
    parent.append(element("p", stats.replay_measured_calls + " of " + stats.flagged_calls + " flagged calls with a measured replay", "muted"));
    if (stats.replay_measured_calls) {
      parent.append(element("p", stats.replay_changed_calls + " with a match-rate change · " +
        stats.joint_replay_measured_calls + " with a measured joint edit", "muted"));
    }
  }
}

function renderAssessmentPatterns(page, back) {
  const data = page.data;
  const shadow = data.kind.startsWith("shadow_");
  const screening = data.kind.startsWith("screening_");
  const tool = screening ? "query_screening_patterns" : shadow ? "query_shadow_patterns" : "query_candidate_patterns";
  const appendStats = screening ? appendScreeningStats : (parent, stats, label) => appendProbeStats(parent, stats, label, shadow);
  if (data.kind.endsWith("_groups")) {
    result.append(element("p", countLabel(data.total_groups, screening ? "source pair" : "probe setup"), "muted"));
    for (const group of data.groups) {
      const row = element("section", undefined, "row");
      const title = screening ? screeningGroupName(group) : probeGroupName(group.setup);
      row.append(button(title, () => requestPage(tool,
        { ...page.args, group: group.key, offset: 0, compare_offset: 0 }, title, { page, previous: back })));
      const method = screening ? [group.scorer?.model_id, group.scorer?.revision] :
        shadow ? [group.setup.runner_id, group.setup.match_rule.replaceAll("_", " ")] :
        [group.setup.model_id, group.setup.model_revision];
      if (method.some(Boolean)) row.append(element("p", method.join(" · "), "muted"));
      appendStats(row, group.primary, data.compared ? "Scope A" : "");
      if (data.compared) appendStats(row, group.comparison, "Scope B");
      result.append(row);
    }
    pageControls(page, tool, back);
    return;
  }
  const sides = data.compared ? [data.primary, data.comparison] : [data.primary];
  sides.forEach((side, index) => {
    const section = element("section", undefined, "row");
    const stats = index ? data.group.comparison : data.group.primary;
    appendStats(section, stats, data.compared ? "Scope " + (index ? "B" : "A") : "");
    for (const item of side.rows) {
      const label = item.conversation_id || item.run_id;
      section.append(button(label, () => requestPage("get_turn_evidence", { run_id: item.run_id, call_id: item.call_id }, label, { page, previous: back })),
        element("p", [item.started_at, item.organization_id, item.user_id, item.context_id].filter(Boolean).join(" · "), "muted"));
      if (screening) {
        const pair = element("details");
        pair.append(element("summary", screeningPairLabel(item)));
        appendScreeningPair(pair, item.assessment, item.sources);
        for (const { decision_id, coverage } of item.replay_decisions || []) {
          let label = "Replay effect unavailable";
          if (coverage.measured) {
            label = coverage.changed ? "Replay changed the match rate" : "Replay match rate unchanged";
            if (coverage.joint_measured) label += " · joint edit measured";
          }
          pair.append(element("p", label),
            button("Open replay on " + decision_id, () => requestPage("get_turn_evidence", {
              run_id: item.run_id, call_id: item.call_id, decision_id,
            }, decision_id, { page, previous: back })));
        }
        section.append(pair);
      } else {
        section.append(element("p", shadow ? shadowCounts(item.baseline) + " → " + shadowCounts(item.edited) + " · " + proposalShift(item.difference_pp)
          : signed.format(item.baseline_log_odds) + " → " + signed.format(item.edited_log_odds) + " log odds"));
      }
      const path = element("details");
      path.append(element("summary", screening ? "Local pair link" : "Local source link"), element("code", item.inspector_path));
      section.append(path);
    }
    pageControls(page, tool, back, {
      container: section, offsetKey: index ? "compare_offset" : "offset", next: side.next_offset,
    });
    result.append(section);
  });
}

function screeningPairLabel(item) {
  return item.suggestion?.replaceAll("_", " ") || (item.assessment.status === "scored" ? "No pair flagged" : item.assessment.reason.replaceAll("_", " "));
}

function appendScreeningPair(parent, assessment, inputs) {
  for (const input of inputs) {
    parent.append(element("p", [input.source_id || input.context_id, input.role, input.anchor?.field].filter(Boolean).join(" · ")));
    appendExcerpt(parent, input.excerpt);
  }
  if (assessment.status === "scored") {
    const scores = new Intl.NumberFormat(undefined, {style: "percent", maximumFractionDigits: 1});
    const format = (direction) => Object.entries(direction).map(([name, value]) => name + " " + scores.format(value)).join(" · ");
    parent.append(element("p", "First → second · " + format(assessment.left_to_right), "muted"),
      element("p", "Second → first · " + format(assessment.right_to_left), "muted"));
  }
}

function renderTurn(page) {
  const { counts, decision } = page.data;
  if (decision?.label && decision.label !== page.title) result.append(element("p", decision.label, "muted"));
  result.append(element("p", [
    countLabel(counts.rule_findings, "finding"),
    countLabel(counts.context_links, "context link"),
    countLabel(counts.replay_tests, "replay test"),
  ].join(" · "), "muted"));
  renderRequestEvidence(page, "get_turn_evidence");
}

function renderPending(page) {
  const { model, captured_inputs, preflight } = page.data;
  result.append(element("p", [model, countLabel(captured_inputs, "captured input")].filter(Boolean).join(" · "), "muted"));
  if (preflight.receipt) {
    const receipt = element("details", undefined, "row");
    receipt.append(element("summary", "Preflight receipt"), element("pre", JSON.stringify(preflight.receipt, null, 2), "receipt-json"));
    result.append(receipt);
  }
  renderRequestEvidence(page, "inspect_pending");
}

function renderRequestEvidence(page, tool) {
  const pending = tool === "inspect_pending";
  const { rows, preflight, context_screening: screening, candidate_probes = [], shadow_probes = [] } = page.data;
  if (preflight) {
    const summary = pending ? countLabel(preflight.finding_count, "finding") :
      preflight.status === "matched" ? countLabel(preflight.finding_count, "finding") + " from the workflow's preflight check" : preflight.issue;
    result.append(element("p", "Preflight · " + summary, "muted"));
  }
  if (screening) {
    const summary = screening.status === "matched" ?
      countLabel(screening.flagged_pairs, "pair") + " to review · " + screening.compared_pairs + " of " + screening.total_pairs + " pairs screened" :
      screening.issue;
    result.append(element("p", "Context screening · " + summary, "muted"));
  }
  const canQuery = !pending || Boolean(page.args.input);
  if (canQuery && page.data.captured_excerpts_enabled && !page.args.include_excerpts) {
    result.append(button("Show captured text", () => requestPage(
      tool, { ...page.args, include_excerpts: true }, page.title, page.back,
    )));
  }
  if (page.data.decision?.outcome_excerpt) appendExcerpt(result, page.data.decision.outcome_excerpt);
  for (const probe of candidate_probes) {
    const section = element("section", undefined, "row");
    section.append(element("h2", "Candidate probe" + (probe.report_id ? " · " + probe.report_id : "")));
    if (probe.status === "unverified") section.append(element("p", probe.issue));
    else {
      section.append(element("p", "Original request · " + candidatePreference(probe.candidate_ids, probe.baseline_log_odds)),
        element("p", probe.model_id + " · " + probe.runtime, "muted"));
      if (probe.prefix_excerpt && probe.candidate_excerpts) {
        const setup = element("details");
        setup.append(element("summary", "Fixed prefix and candidates"));
        appendExcerpt(setup, probe.prefix_excerpt);
        for (const candidate of probe.candidate_excerpts) appendExcerpt(setup, candidate);
        section.append(setup);
      }
    }
    result.append(section);
  }
  for (const probe of shadow_probes) {
    const section = element("section", undefined, "row");
    section.append(element("h2", "Shadow probe" + (probe.report_id ? " · " + probe.report_id : "")));
    if (probe.status === "unverified") section.append(element("p", probe.issue));
    else {
      section.append(element("p", "Original request · " + shadowCounts(probe.baseline)),
        element("p", probe.runner_id, "muted"));
      appendExcerpt(section, probe.expected_label);
      appendExcerpt(section, probe.expected_outcome);
      if (probe.pair) {
        section.append(element("p", "Joint edit · " + shadowCounts(probe.pair.summary) + " · " + proposalShift(probe.pair.difference_pp)),
          element("p", "Interaction · " + proposalShift(probe.pair.interaction_pp), "muted"));
      }
      if (probe.control_comparison) {
        section.append(element("p", "Target vs control · " + proposalShift(probe.control_comparison.target_minus_control_pp)));
      }
    }
    result.append(section);
  }
  const measured = (page.data.source_visibility_reports || []).filter(({ measured_continuations }) => measured_continuations > 0);
  if (measured.length) {
    const section = element("section", undefined, "row");
    section.append(element("h2", "Masked replays"));
    for (const report of measured) {
      section.append(element("p", report.model_id + " · " + report.runtime, "muted"));
      section.append(button("Open " + countLabel(report.measured_continuations, "saved replay"), () => requestPage(
        "query_source_visibility", {
          version: 1, location: turnLocation(page.data), report_id: report.report_id, view: { kind: "continuations" },
        }, "Decoding with sources masked", { page, previous: page.back },
      )));
    }
    result.append(section);
  }
  const calls = element("ol");
  const inputs = element("details", undefined, "row");
  inputs.append(element("summary", "Captured inputs on this page"));
  for (const item of rows) {
    const row = element("section", undefined, "row");
    if (item.kind === "rule_finding" || item.kind === "preflight_finding") {
      const phase = item.kind === "preflight_finding" ? "Preflight · " : "";
      row.append(element("h2", phase + [item.finding.replaceAll("_", " "), item.rule, item.scope].join(" · ")));
      for (const input of item.inputs) {
        row.append(element("p", [input.source_id || input.context_id, input.source_version, input.anchor?.field].filter(Boolean).join(" · ")));
        if (input.rule_value != null) row.append(element("p", JSON.stringify(input.rule_value)));
        appendExcerpt(row, input.excerpt);
      }
    } else if (item.kind === "candidate_probe_effect") {
      row.append(element("h2", [item.source_id || item.context_id, item.source_version].filter(Boolean).join(" · ")),
        element("p", "After edit · " + candidatePreference(item.candidate_ids, item.log_odds)),
        element("p", "Shift " + signed.format(item.change_log_odds) + " log odds", "muted"));
      const range = item.text_range ? " · UTF-8 bytes " + item.text_range.start_byte + " to " + item.text_range.end_byte : "";
      row.append(element("small", item.pointer + range));
      appendExcerpt(row, item.replacement_excerpt);
    } else if (item.kind === "context_screen_pair") {
      row.append(element("h2", "Context screen · " + screeningPairLabel(item)));
      appendScreeningPair(row, item.assessment, item.inputs);
    } else if (item.kind === "shadow_probe_effect") {
      const range = item.text_range ? " · UTF-8 bytes " + item.text_range.start_byte + " to " + item.text_range.end_byte : "";
      row.append(element("h2", "Shadow edit · " + [item.source_id || item.context_id, item.source_version].filter(Boolean).join(" · ")),
        element("p", "After edit · " + shadowCounts(item.summary)),
        element("p", "Shift · " + proposalShift(item.difference_pp), "muted"),
        element("small", item.pointer + range));
      appendExcerpt(row, item.replacement_excerpt);
    } else if (item.kind === "context_link") {
      row.append(element("h2", "Context link"), element("p", [item.label || item.relationship, item.context_id].filter(Boolean).join(" · ")));
      if (item.excerpt) {
        const detail = element("details");
        detail.append(element("summary", "Recorded excerpt"));
        appendExcerpt(detail, item.excerpt);
        row.append(detail);
      }
    } else if (item.kind === "replay_test") {
      const change = item.result.difference_pp ?? item.result.target_minus_control_pp;
      row.append(element("h2", "Replay · " + item.test), element("p", change == null ? "Effect unavailable" : change + " percentage points"));
      const sources = item.result.context_ids || [item.result.context_id].filter(Boolean);
      if (sources.length) row.append(element("p", sources.join(" · "), "muted"));
    } else if (item.kind === "recorded_call") {
      calls.append(element("li", item.call_id));
      continue;
    } else if (item.kind === "recorded_decision") {
      const location = { ...turnLocation(page.data), decision_id: item.decision_id };
      row.append(button("Open " + item.label, () => requestPage(
        "get_turn_evidence", { ...location, include_excerpts: page.args.include_excerpts ?? false },
        item.label, { page, previous: page.back },
      )));
    } else if (item.input) {
      inputs.append(element("p", [item.input.source_id || item.input.context_id, item.input.availability, item.input.anchor?.field].filter(Boolean).join(" · ")));
      for (const { pointer, text_range } of item.request_fields || []) {
        const range = text_range ? " · UTF-8 bytes " + text_range.start_byte + " to " + text_range.end_byte : "";
        inputs.append(element("p", pointer + range, "muted"));
      }
      appendExcerpt(inputs, item.input.excerpt);
      if (!pending && item.kind === "captured_input" && item.input.source_id?.trim()) {
        const id = item.input.source_id;
        inputs.append(button("View source history for " + id, () => requestPage(
          "query_context_sources", { version: 1, source_id: id }, id, { page, previous: page.back },
        )));
      }
      continue;
    } else {
      continue;
    }
    result.append(row);
  }
  if (calls.childElementCount) {
    const steps = element("section", undefined, "row");
    steps.append(element("h2", "Recorded calls"), calls);
    result.append(steps);
  }
  if (inputs.childElementCount > 1) result.append(inputs);
  if (canQuery) pageControls(page, tool, page.back);
}

function shadowCounts({ attempted, matched, completed, unscored = 0 }) {
  let text = matched + " of " + attempted + " matched";
  if (attempted > completed) text += " · " + countLabel(attempted - completed, "failed trial");
  if (unscored) text += " · " + countLabel(unscored, "unscored trial");
  return text;
}

function proposalShift(value) {
  return value == null ? "comparison unavailable" : signed.format(value) + " pp";
}

function appendExcerpt(parent, excerpt) {
  if (excerpt) parent.append(element("blockquote", excerpt.text + (excerpt.truncated ? "…" : "")));
}

function candidatePreference(ids, margin) {
  const choice = margin === 0 ? "Tie" : "Favors " + ids[margin > 0 ? 0 : 1];
  return choice + " · " + signed.format(margin) + " log odds";
}

const activationTool = "query_activation_effects";
const signed = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2, signDisplay: "exceptZero" });

function activationTitle(group) {
  const source = group.source_id || "Unnamed source";
  return source.replaceAll("-", " ") + " at " + group.checkpoint;
}

function activationStats(parent, stats) {
  const { minimum, maximum } = stats.change_log_odds;
  const range = minimum === maximum ? signed.format(minimum) : signed.format(minimum) + " to " + signed.format(maximum);
  parent.append(element("p", range + " log odds · "
    + stats.lower_score_count + "/" + stats.decision_count + " lower · "
    + stats.flipped_count + "/" + stats.decision_count + " flipped", "muted"));
}

function activationScopeStats(parent, stats) {
  parent.append(element("p", countLabel(stats.measurement_count, "measurement") + " · "
    + countLabel(stats.conversation_count, "identified conversation")));
  activationStats(parent, stats);
}

function activationDetails(parent, group) {
  parent.append(element("small", group.model_id + " · " + group.runtime + " · token " + (group.output_token + 1)));
  if (group.contrasts) {
    const { before, after, alternatives } = group.contrasts;
    const text = (excerpt) => excerpt.text + (excerpt.truncated ? "…" : "");
    parent.append(element("p", text(before) + " → " + text(after)
      + " · " + alternatives.map(text).join(" vs. ")));
  }
}

function activationExcerptsButton(page, back) {
  if (page.args.version === 1 && page.data.captured_excerpts_enabled && !page.args.include_excerpts) {
    result.append(button("Show tested values", () => requestPage(
      activationTool, { ...page.args, include_excerpts: true }, page.title, back,
    )));
  }
}

function renderActivationGroups(page, back) {
  result.append(element("p", countLabel(page.data.total_groups, "comparable group")
    + " · decoder block " + page.data.block, "muted"));
  if (page.args.version === 1) {
    const controls = element("div", undefined, "actions activation-controls");
    const label = element("label", "Decoder block ");
    const block = element("input");
    block.type = "number";
    block.min = "0";
    block.step = "1";
    block.required = true;
    block.value = page.data.block;
    label.append(block);
    const apply = button("Compare block", () => {
      if (!block.reportValidity()) return;
      requestPage(activationTool, { ...page.args, block: block.valueAsNumber, offset: 0 }, page.title, back);
    });
    block.addEventListener("keydown", (event) => {
      if (event.key === "Enter") apply.click();
    });
    controls.append(label, apply);
    result.append(controls);
  }
  activationExcerptsButton(page, back);
  for (const group of page.data.groups) {
    const row = element("section", undefined, "row");
    const title = activationTitle(group);
    if (page.args.version === 1) {
      row.append(button(title, () => requestPage(
        activationTool, { ...page.args, group: group.key, offset: 0, compare_offset: 0 }, title, { page, previous: back },
      )));
    } else {
      row.append(element("h2", title));
    }
    if (page.kind === "activation_group_comparison") {
      scopeColumns(row, page.args, [group.scope_a, group.scope_b], activationScopeStats, "No measurements");
    } else {
      activationStats(row, group);
    }
    activationDetails(row, group);
    result.append(row);
  }
  if (page.args.version === 1) pageControls(page, activationTool, back);
}

function renderActivationEvidence(page, back) {
  const { data } = page;
  const compared = page.kind === "activation_evidence_comparison";
  result.append(element("p", (compared ? "" : countLabel(data.total_occurrences, "measurement") + " · ")
    + "Decoder block " + data.block, "muted"));
  if (!compared) activationStats(result, data.group);
  activationDetails(result, data.group);
  activationExcerptsButton(page, back);
  const { source_id, source_version } = page.data.group;
  if (source_id) {
    const selection = Object.fromEntries(["scope", "thread", "after", "before", "compare"]
      .filter((key) => Object.hasOwn(page.args, key)).map((key) => [key, page.args[key]]));
    result.append(button("View source history for " + source_id, () => requestPage(
      "query_context_sources", { version: 1, source_id, source_version, ...selection, evidence_filter: "activation_swaps" },
      source_id, { page, previous: back },
    )));
  }
  if (compared) {
    const sides = [data.scope_a, data.scope_b].map((side) => side.total_occurrences ? side : null);
    scopeColumns(result, page.args, sides, (section, side, index) => {
      activationScopeStats(section, index ? data.group.scope_b : data.group.scope_a);
      renderActivationOccurrences(section, side, page, back, index);
    }, "No measurements");
  } else {
    renderActivationOccurrences(result, data, page, back, 0);
  }
}

function renderActivationOccurrences(parent, side, page, back, index) {
  for (const item of side.occurrences) {
    const row = element("section", undefined, "row");
    const title = item.conversation_id || item.location.run_id;
    row.append(button(title, () => requestPage(
      "get_turn_evidence", item.location, title, { page, previous: back },
    )));
    row.append(element("p", signed.format(item.original_log_odds) + " → "
      + signed.format(item.patched_log_odds) + " log odds", "muted"));
    const owner = [item.organization_id, item.user_id, item.context_id].filter(Boolean).join(" · ");
    if (owner) row.append(element("p", owner, "muted"));
    if (item.started_at) row.append(element("p", item.started_at, "muted"));
    const paths = element("details");
    paths.append(element("summary", "Local inspector paths"));
    paths.append(element("p", "Checkpoint"), element("code", item.inspector_path));
    paths.append(element("p", "Source"), element("code", item.source_inspector_path));
    row.append(paths);
    parent.append(row);
  }
  if (page.args.version === 1) pageControls(page, activationTool, back, {
    container: parent, offsetKey: index ? "compare_offset" : "offset", next: side.next_offset,
  });
}

const replayEffectsTool = "query_replay_effects";

function replayEffectTitle(group) {
  const { kind, edits } = group.setup;
  const names = edits.map((edit) => [edit.source_id || edit.context_role || edit.context_kind, edit.source_version]
    .filter(Boolean).join(" · "));
  return names.join(kind === "control" ? " vs " : " + ");
}

function replayEffectStats(parent, stats) {
  const { min_difference_pp: min, max_difference_pp: max } = stats;
  const range = min == null ? "No usable effect" : (min === max ? signed.format(min) :
    signed.format(min) + " to " + signed.format(max)) + " pp";
  const counts = ["lower", "unchanged", "higher", "mixed", "unavailable"]
    .filter((key) => stats[key]).map((key) => stats[key] + " " + key);
  parent.append(element("p", range + " · " + counts.join(" · ")),
    element("p", [countLabel(stats.decisions, "decision"), countLabel(stats.conversations, "identified conversation"),
      stats.verified_baselines + " of " + stats.measurements + " baselines verified"].join(" · "), "muted"));
}

function replayEffectDetails(parent, group) {
  const { kind, model, runner_id, decision_label, match_rule, sampling_design, edits } = group.setup;
  const method = { single: "Single edit", pair: "Joint edit", control: "Target minus control" }[kind];
  const sampling = sampling_design === "independent" ? "Independent sampling" : null;
  parent.append(element("p", [method, decision_label, model, runner_id, match_rule.replaceAll("_", " "), sampling]
    .filter(Boolean).join(" · "), "muted"));
  edits.forEach((edit) => appendExcerpt(parent, edit.replacement_excerpt));
}

function renderReplayEffects(page, back) {
  const { data, args } = page;
  const canQuery = args.version === 1;
  const scopes = { version: args.version, ...args.selection, compare: args.compare };
  if (canQuery && data.captured_excerpts_enabled && !args.include_excerpts) {
    result.append(button("Show tested edits", () => requestPage(
      replayEffectsTool, { ...args, include_excerpts: true }, page.title, back,
    )));
  }
  if (data.kind === "replay_effect_groups") {
    result.append(element("p", countLabel(data.total_groups, "comparable replay group"), "muted"));
    for (const group of data.groups) {
      const row = element("section", undefined, "row");
      const title = replayEffectTitle(group);
      row.append(canQuery ? button(title, () => requestPage(replayEffectsTool,
        { ...args, group: group.key, offset: 0, compare_offset: 0 }, title, { page, previous: back })) : element("h2", title));
      replayEffectDetails(row, group);
      if (data.compared) scopeColumns(row, scopes, [group.primary, group.comparison], replayEffectStats, "No comparable replays");
      else replayEffectStats(row, group.primary);
      result.append(row);
    }
    if (canQuery) pageControls(page, replayEffectsTool, back);
    return;
  }
  replayEffectDetails(result, data.group);
  const renderSide = (parent, side, index) => {
    const stats = index ? data.group.comparison : data.group.primary;
    if (!stats) {
      parent.append(element("p", "No comparable replays", "muted"));
      return;
    }
    replayEffectStats(parent, stats);
    for (const item of side.rows) {
      const row = element("section", undefined, "row");
      const title = item.conversation_id || item.location.run_id;
      const trialCount = ({ matched, completed, unscored = 0, attempted }) =>
        matched + "/" + (completed - unscored) + " matched" + (attempted === completed - unscored ? "" : " of " + attempted + " attempted");
      const effect = item.difference_pp == null ? "Effect unavailable" : signed.format(item.difference_pp) + " pp";
      row.append(button(title, () => requestPage("get_turn_evidence", item.location, title, { page, previous: back })),
        element("p", "Baseline " + trialCount(item.baseline) + " · " + (item.control ? "Target " : "Edited ") + trialCount(item.variant)
          + (item.control ? " · Control " + trialCount(item.control) : "") + " · " + effect),
        element("p", [item.started_at, item.organization_id, item.user_id, item.turn_id].filter(Boolean).join(" · "), "muted"));
      const path = element("details");
      path.append(element("summary", "Local replay link"), element("code", item.inspector_path));
      parent.append(row, path);
    }
    if (canQuery) pageControls(page, replayEffectsTool, back, {
      container: parent, offsetKey: index ? "compare_offset" : "offset", next: side.next_offset,
    });
  };
  if (data.compared) scopeColumns(result, scopes, [data.primary, data.comparison], renderSide, "No comparable replays");
  else renderSide(result, data.primary, 0);
}

app.ontoolinput = ({ arguments: args }) => { initialQuery = args; };
app.ontoolresult = (response) => {
  try {
    generation += 1;
    const data = payload(response);
    const turn = data.kind === "turn_evidence";
    const args = turn ? { ...turnLocation(data), ...initialQuery } : initialQuery || {};
    const titles = {
      groups: () => data.phase === "preflight" ? "Conflicts found before inference" : "Conflicts across recorded decisions",
      evidence: () => args.group ? groupName(args.group, args.group_by || [])
        : data.phase === "preflight" ? "Contributing checks" : "Contributing decisions",
      activation_groups: () => "Recurring activation effects",
      activation_group_comparison: () => "Compare activation effects",
      activation_evidence: () => activationTitle(data.group),
      activation_evidence_comparison: () => activationTitle(data.group),
      candidate_pattern_groups: () => "Candidate preference shifts",
      candidate_pattern_evidence: () => probeGroupName(data.group.setup),
      shadow_pattern_groups: () => "Shadow proposal shifts",
      shadow_pattern_evidence: () => probeGroupName(data.group.setup),
      screening_pattern_groups: () => "Screened context pairs",
      screening_pattern_evidence: () => screeningGroupName(data.group),
      replay_effect_groups: () => data.compared ? "Compare replay effects" : "Recurring replay effects",
      replay_effect_evidence: () => replayEffectTitle(data.group),
      pending_evidence: () => "Before inference",
      turn_evidence: () => "Turn evidence",
    };
    const title = Object.hasOwn(titles, data.kind) ? titles[data.kind]() : sourceHistoryTitle(data, args) ?? visibilityTitle(data);
    if (!title) throw new Error("This result has no supported evidence view.");
    openPage({ kind: turn ? "turn" : data.kind, title, args, data });
  } catch (error) {
    showStatus(error.message, true);
  }
};
app.ontoolcancelled = () => {
  generation += 1;
  showStatus("Query cancelled.");
};
function updateTheme({ theme } = {}) {
  if (theme === "light" || theme === "dark") document.documentElement.style.colorScheme = theme;
}
app.onhostcontextchanged = updateTheme;
app.connect().then(() => updateTheme(app.getHostContext())).catch(() => showStatus("Could not connect to the host.", true));
