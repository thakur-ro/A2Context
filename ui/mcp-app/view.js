import { App } from "@modelcontextprotocol/ext-apps";
import { button, countLabel, element } from "./dom.js";
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
    const label = visibilityBackLabel(back.page.kind) ?? (back.page.kind === "turn" ? "Back to turn evidence" :
      page.kind === "turn" ? "Back to contributing decisions" :
      page.kind === "activation_evidence" ? "Back to activation groups" : "Back to conflict groups");
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
  else if (page.kind === "activation_groups") renderActivationGroups(page, back);
  else if (page.kind === "activation_evidence") renderActivationEvidence(page, back);
  else if (page.kind.startsWith("visibility_")) renderVisibility(result, page, requestPage, pageControls);
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
  result.append(element("p", totals.conflicting_decisions + " conflicting decisions · "
    + totals.comparable_decisions + " comparable · " + totals.scoped_decisions + " scoped", "muted"));
  for (const { key, conflicting_decisions, affected_conversations } of rows) {
    const name = groupName(key, page.args.group_by || []);
    const row = element("div", undefined, "row");
    if (conflicting_decisions && page.args.group_by) {
      row.append(button(name, () => requestPage("query_conflicts", { ...page.args, group: key, offset: 0 }, name, { page, previous: back })));
    } else {
      row.append(element("h2", name));
    }
    row.append(element("p", countLabel(conflicting_decisions, "conflict") + " · "
      + countLabel(affected_conversations, "identified conversation"), "muted"));
    result.append(row);
  }
  if (!rows.length) result.append(element("p", "No scoped decisions match this view.", "muted"));
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
    for (const { run_id, call_id, decision_id, decision_label, turn_id } of rows) {
      const label = [decision_label, turn_id, call_id].filter(Boolean).join(" · ");
      const control = button(label, () => requestPage(
        "get_turn_evidence", { run_id, call_id, decision_id }, decision_label, { page, previous: back },
      ));
      const entry = element("div");
      entry.append(control);
      section.append(entry);
    }
    result.append(section);
  }
  if (!groups.size) result.append(element("p", "No conflicting decisions in this group.", "muted"));
  if (page.args.group_by) pageControls(page, "query_conflicts", back);
}

function turnLocation({ run_id, call_id, decision }) {
  return { run_id, call_id, decision_id: decision?.id };
}

function renderTurn(page) {
  const { counts, rows, preflight } = page.data;
  result.append(element("p", [
    countLabel(counts.rule_findings, "finding"),
    countLabel(counts.context_links, "context link"),
    countLabel(counts.replay_tests, "replay test"),
  ].join(" · "), "muted"));
  if (preflight) {
    const summary = preflight.status === "matched"
      ? countLabel(preflight.finding_count, "finding") + " from the workflow's preflight check"
      : preflight.issue;
    result.append(element("p", "Preflight · " + summary, "muted"));
  }
  if (page.data.captured_excerpts_enabled && !page.args.include_excerpts) {
    result.append(button("Show captured text", () => requestPage(
      "get_turn_evidence", { ...page.args, include_excerpts: true }, page.title, page.back,
    )));
  }
  if (page.data.decision?.outcome_excerpt) appendExcerpt(result, page.data.decision.outcome_excerpt);
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
    } else if (item.kind === "recorded_call") {
      calls.append(element("li", item.call_id));
      continue;
    } else if (item.input) {
      inputs.append(element("p", [item.input.source_id || item.input.context_id, item.input.availability, item.input.anchor?.field].filter(Boolean).join(" · ")));
      appendExcerpt(inputs, item.input.excerpt);
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
  pageControls(page, "get_turn_evidence", page.back);
}

function appendExcerpt(parent, excerpt) {
  if (excerpt) parent.append(element("blockquote", excerpt.text + (excerpt.truncated ? "…" : "")));
}

const activationTool = "query_activation_effects";
const signed = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2, signDisplay: "exceptZero" });

function activationTitle(group) {
  const source = group.source_id || "Unnamed source";
  return source.replaceAll("-", " ") + " at " + group.checkpoint;
}

function activationSummary(parent, group) {
  const { minimum, maximum } = group.change_log_odds;
  const range = minimum === maximum ? signed.format(minimum) : signed.format(minimum) + " to " + signed.format(maximum);
  parent.append(element("p", range + " log odds · "
    + group.lower_score_count + "/" + group.decision_count + " lower · "
    + group.flipped_count + "/" + group.decision_count + " flipped", "muted"));
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
        activationTool, { ...page.args, group: group.key, offset: 0 }, title, { page, previous: back },
      )));
    } else {
      row.append(element("h2", title));
    }
    activationSummary(row, group);
    result.append(row);
  }
  if (page.args.version === 1) pageControls(page, activationTool, back);
}

function renderActivationEvidence(page, back) {
  result.append(element("p", countLabel(page.data.total_occurrences, "measurement")
    + " · decoder block " + page.data.block, "muted"));
  activationSummary(result, page.data.group);
  activationExcerptsButton(page, back);
  for (const item of page.data.occurrences) {
    const row = element("section", undefined, "row");
    const title = item.conversation_id || item.location.run_id;
    row.append(button(title, () => requestPage(
      "get_turn_evidence", item.location, title, { page, previous: back },
    )));
    row.append(element("p", signed.format(item.original_log_odds) + " → "
      + signed.format(item.patched_log_odds) + " log odds", "muted"));
    const owner = [item.organization_id, item.user_id, item.context_id].filter(Boolean).join(" · ");
    if (owner) row.append(element("p", owner, "muted"));
    const paths = element("details");
    paths.append(element("summary", "Local inspector paths"));
    paths.append(element("p", "Checkpoint"), element("code", item.inspector_path));
    paths.append(element("p", "Source"), element("code", item.source_inspector_path));
    row.append(paths);
    result.append(row);
  }
  if (page.args.version === 1) pageControls(page, activationTool, back);
}

app.ontoolinput = ({ arguments: args }) => { initialQuery = args; };
app.ontoolresult = (response) => {
  try {
    generation += 1;
    const data = payload(response);
    const turn = data.kind === "turn_evidence";
    const args = turn ? { ...turnLocation(data), ...initialQuery } : initialQuery || {};
    const titles = {
      groups: () => "Conflicts across recorded decisions",
      evidence: () => args.group ? groupName(args.group, args.group_by || []) : "Contributing decisions",
      activation_groups: () => "Recurring activation effects",
      activation_evidence: () => activationTitle(data.group),
      turn_evidence: () => "Turn evidence",
    };
    const title = Object.hasOwn(titles, data.kind) ? titles[data.kind]() : visibilityTitle(data);
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
