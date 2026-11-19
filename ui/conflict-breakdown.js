"use strict";

const conflictDimensions = new Map([
  ["rule", "Rule"],
  ["source_version", "Source and version"],
  ["model", "Model"],
  ["organization", "Organization"],
  ["user", "User"],
  ["conversation", "Conversation"],
  ["day", "Day (UTC)"],
  ["trace_source", "Trace source"],
]);
let conflictGroup = null;

function setupConflictBreakdown() {
  const primary = element("conflict-group-by");
  const secondary = element("conflict-then-by");
  secondary.add(new Option("None", ""));
  for (const [key, label] of conflictDimensions) {
    primary.add(new Option(label, key));
    secondary.add(new Option(label, key));
  }
  for (const id of ["conflict-group-by", "conflict-then-by", "conflict-rule"]) {
    element(id).addEventListener("change", () => {
      conflictGroup = null;
      linkNotice();
      renderPatterns();
    });
  }
}

function openConflictBreakdown(run, rule) {
  state.view = "patterns";
  state.patternThread = "";
  state.patternScope = JSON.stringify([run.organization_id, run.user_id]);
  element("pattern-type").value = "breakdown";
  element("conflict-group-by").value = "source_version";
  element("conflict-then-by").value = "";
  element("conflict-rule").value = rule;
  conflictGroup = null;
  linkNotice();
  render();
  element("conflict-group-by").focus();
}

function writeConflictBreakdownLink(params) {
  params.set("group_by", element("conflict-group-by").value);
  if (element("conflict-then-by").value) params.set("then_by", element("conflict-then-by").value);
  if (element("conflict-rule").value.trim()) params.set("rule", element("conflict-rule").value.trim());
  if (conflictGroup) params.set("group", JSON.stringify(conflictGroup));
}

function restoreConflictBreakdownLink(params) {
  selectLinkValue("conflict-group-by", conflictDimensions.has(params.get("group_by")) ? params.get("group_by") : "rule");
  selectLinkValue("conflict-then-by", conflictDimensions.has(params.get("then_by")) ? params.get("then_by") : "");
  element("conflict-rule").value = params.get("rule") || "";
  conflictGroup = null;
  try {
    const key = JSON.parse(params.get("group"));
    if (Array.isArray(key) && key.every((part) => Array.isArray(part)
      && part.every((value) => value === null || typeof value === "string"))) conflictGroup = key;
  } catch {
    // An invalid group leaves the view open at its top-level results.
  }
}

function conflictGroupLabel(key, dimensions) {
  return key.map((part, index) => {
    const dimension = dimensions[index];
    if (dimension === "conversation") {
      const [source, org, user, thread] = part;
      return [thread || "Conversation ID unavailable", org, user, source].filter(Boolean).join(" / ");
    }
    if (dimension === "user") {
      const [org, user] = part;
      return [user || "User ID unavailable", org].filter(Boolean).join(" / ");
    }
    if (dimension === "source_version") {
      return [part[0] || "Unnamed source", part[1] || "Version unavailable"].join(" · ");
    }
    return part.filter(Boolean).join(" · ") || conflictDimensions.get(dimension) + " unavailable";
  }).join(" → ");
}

async function showConflictPage(container, spec, renderPage) {
  const pending = node("p", "Loading conflict evidence…", "muted");
  pending.setAttribute("role", "status");
  container.replaceChildren(pending);
  try {
    const response = await fetch("/api/conflict-query", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(spec),
    });
    const page = await response.json();
    if (!response.ok) throw new Error(page.message || "Could not load conflict evidence.");
    if (!pending.isConnected) return;
    container.replaceChildren();
    renderPage(page);
  } catch (error) {
    pending.textContent = error.message;
  }
}

function conflictPager(container, offset, next, load) {
  if (!offset && next === null) return;
  const nav = node("div", undefined, "breakdown-pages");
  for (const [label, target] of [["Previous page", offset ? Math.max(0, offset - 20) : null], ["Next page", next]]) {
    const button = node("button", label);
    button.type = "button";
    button.disabled = target === null;
    button.addEventListener("click", () => load(target));
    nav.append(button);
  }
  container.append(nav);
}

function renderConflictBreakdown(list, scope) {
  const first = element("conflict-group-by").value;
  const second = element("conflict-then-by");
  for (const option of second.options) option.disabled = option.value === first;
  if (second.value === first) second.value = "";
  const group_by = [first, second.value].filter(Boolean);
  const { organization, user } = scope;
  const rules = new Set();
  for (const run of data.runs) {
    if ((organization !== null || user !== null) && run.organization_id !== organization) continue;
    if (user !== null && run.user_id !== user) continue;
    for (const call of run.calls) for (const { applies_to } of call.decisions) {
      if (applies_to) rules.add(applies_to.key);
    }
  }
  element("conflict-rules").replaceChildren(...[...rules].sort().map((key) => new Option(key)));
  const rule = element("conflict-rule").value.trim();
  const spec = {
    version: 1,
    scope: user !== null ? { kind: "user", organization_id: organization, user_id: user }
      : organization !== null ? { kind: "organization", organization_id: organization } : { kind: "workspace" },
    group_by,
    filters: rule ? { rule } : {},
    limit: 20,
  };
  const results = node("section", undefined, "panel breakdown-results");
  const evidence = node("section", undefined, "panel breakdown-evidence");
  evidence.id = "conflict-evidence";
  evidence.hidden = true;
  list.append(results, evidence);

  function loadEvidence(group, offset = 0, focus = false) {
    conflictGroup = group;
    for (const button of results.querySelectorAll("button[data-group-key]")) {
      button.setAttribute("aria-expanded", String(button.dataset.groupKey === JSON.stringify(group)));
    }
    evidence.hidden = false;
    showConflictPage(evidence, { ...spec, group, offset }, ({ rows, next_offset }) => {
      const heading = node("h2", conflictGroupLabel(group, group_by));
      heading.tabIndex = -1;
      evidence.append(heading);
      for (const entries of groupConflictConversations(rows).values()) {
        const [first] = entries;
        const title = [first.conversation_id || "Conversation ID unavailable", first.user_id, first.organization_id].filter(Boolean);
        if (!first.conversation_id) title.push(first.run_id);
        evidence.append(node("h3", title.join(" · ")));
        const labels = entries.map(({ decision_label }) => decision_label);
        for (const row of entries) {
          const entry = node("div", undefined, "breakdown-contributor");
          entry.append(node("span", row.decision_label), conversationOccurrenceButton(row));
          const repeated = labels.indexOf(row.decision_label) !== labels.lastIndexOf(row.decision_label);
          const position = [row.started_at, row.turn_id, repeated && row.call_id].filter(Boolean);
          if (position.length) entry.append(node("small", position.join(" · "), "muted"));
          evidence.append(entry);
        }
      }
      if (!rows.length) evidence.append(node("p", "No conflicting decisions in this group.", "muted"));
      conflictPager(evidence, offset, next_offset, (next) => loadEvidence(group, next, true));
      if (focus && state.view === "patterns") heading.focus();
    });
  }

  function loadGroups(offset = 0, focus = false) {
    showConflictPage(results, { ...spec, offset }, ({ totals, rows, next_offset }) => {
      const heading = node("h2", "Conflicts across recorded decisions");
      heading.tabIndex = -1;
      results.append(heading);
      results.append(node("p", totals.conflicting_decisions + " with conflicts · "
        + totals.comparable_decisions + " comparable · " + totals.scoped_decisions + " with declared rule scope", "count"));
      const missing = totals.conflicts_without_conversation;
      results.append(node("p", countLabel(totals.affected_conversations, "identified conversation") + " with conflicts"
        + (missing ? " · " + countLabel(missing, "conflict") + " without a conversation ID" : ""), "muted"));
      if (!rows.length) {
        results.append(node("p", "No decisions with declared rule scope match this view.", "muted"));
        return;
      }
      const table = node("table", undefined, "breakdown-table");
      const header = table.createTHead().insertRow();
      for (const label of ["Group", "Conflicts", "Comparable", "Scoped", "Conversations"]) {
        const cell = node("th", label);
        cell.scope = "col";
        header.append(cell);
      }
      const body = table.createTBody();
      for (const row of rows) {
        const line = body.insertRow();
        const label = conflictGroupLabel(row.key, group_by);
        const head = node("th");
        head.scope = "row";
        line.append(head);
        if (row.conflicting_decisions) {
          const button = node("button", label);
          button.type = "button";
          button.dataset.groupKey = JSON.stringify(row.key);
          button.setAttribute("aria-expanded", String(button.dataset.groupKey === JSON.stringify(conflictGroup)));
          button.setAttribute("aria-controls", evidence.id);
          button.addEventListener("click", () => loadEvidence(row.key, 0, true));
          head.append(button);
        } else {
          head.textContent = label;
        }
        for (const count of [
          row.conflicting_decisions, row.comparable_decisions, row.scoped_decisions, row.affected_conversations,
        ]) {
          const cell = line.insertCell();
          cell.textContent = String(count);
        }
      }
      const scroll = node("div", undefined, "breakdown-table-scroll");
      scroll.tabIndex = 0;
      scroll.setAttribute("role", "region");
      scroll.setAttribute("aria-label", "Conflict groups and coverage");
      scroll.append(table);
      results.append(scroll, node("p", "Comparable decisions have at least two captured rule values. A decision can appear in several source groups.", "muted"));
      conflictPager(results, offset, next_offset, (next) => loadGroups(next, true));
      if (focus && state.view === "patterns") heading.focus();
    });
  }
  loadGroups();
  if (conflictGroup) loadEvidence(conflictGroup);
}

function groupConflictConversations(rows) {
  const groups = new Map();
  for (const row of rows) {
    const key = row.conversation_id
      ? JSON.stringify([row.trace_source, row.organization_id, row.user_id, row.conversation_id])
      : JSON.stringify([row.run_id]);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(row);
  }
  return groups;
}
