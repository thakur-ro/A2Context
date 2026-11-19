"use strict";

function matrixToolName(name) {
  const parts = name.split("__");
  if (parts.length < 3 || parts[0] !== "mcp" || parts.some((part) => !part)) return { display: name };
  const tool = parts.pop();
  return { display: tool.replaceAll("_", " "), namespace: parts.slice(1).join("__") };
}

function buildContextMatrix(entries) {
  const sources = new Map();
  const decisions = new Map();
  for (const entry of entries) {
    const { group } = entry;
    const sourceId = JSON.stringify([group.trace_source, group.source]);
    const decisionId = JSON.stringify([group.trace_source, group.decision_kind, group.decision_label]);
    if (!sources.has(sourceId)) {
      const type = group.source.kind;
      const name = type === "tool_definition" ? group.decision_label : group.source.tool;
      const suffix = type === "tool_definition" ? " definition" : " result";
      const toolName = name ? matrixToolName(name) : null;
      sources.set(sourceId, {
        label: name ? name + suffix : entry.source,
        display: toolName ? toolName.display + suffix : entry.source,
        namespace: toolName?.namespace,
        trace: group.trace_source,
        fingerprint: group.source.text_sha256 || group.source.json_sha256,
        cells: new Map(),
        weight: 0,
      });
    }
    if (!decisions.has(decisionId)) {
      const name = ["tool_call", "tool_execution"].includes(group.decision_kind)
        ? matrixToolName(group.decision_label) : { display: group.decision_label };
      decisions.set(decisionId, {
        id: decisionId, label: group.decision_label,
        ...name,
        trace: group.trace_source, kind: labels[group.decision_kind] || group.decision_kind,
        weight: 0,
      });
    }
    const cells = sources.get(sourceId).cells;
    if (!cells.has(decisionId)) cells.set(decisionId, { entries: [] });
    cells.get(decisionId).entries.push(entry);
  }

  for (const source of sources.values()) {
    for (const [decisionId, cell] of source.cells) {
      const matching = cell.entries.filter((entry) => entry.matching.length);
      if (!matching.length) {
        source.cells.delete(decisionId);
        continue;
      }
      cell.occurrences = cell.entries.flatMap((entry) => entry.occurrences);
      cell.decisions = distinctLinkedDecisions(cell.occurrences);
      cell.entries = matching;
      source.weight += cell.decisions;
      decisions.get(decisionId).weight += cell.decisions;
    }
  }
  const byWeight = (left, right) => right.weight - left.weight || left.label.localeCompare(right.label);
  return {
    sources: [...sources.values()].filter((source) => source.weight).sort(byWeight),
    decisions: [...decisions.values()].filter((decision) => decision.weight).sort(byWeight),
  };
}

function matrixNames(items) {
  const names = new Map();
  for (const item of items) names.set(item.display, (names.get(item.display) || 0) + 1);
  return names;
}

function renderContextMatrix(list, entries, status) {
  const matrix = buildContextMatrix(entries);
  const traces = new Set(matrix.sources.map((source) => source.trace));
  const trace = traces.size === 1 ? matrix.sources[0].trace : null;
  const sourceNames = matrixNames(matrix.sources);
  const decisionNames = matrixNames(matrix.decisions);
  const dimensions = [
    { name: "Sources", items: matrix.sources, size: 12, start: 0 },
    { name: "Decisions", items: matrix.decisions, size: 8, start: 0 },
  ];
  const panel = node("div", undefined, "panel context-matrix");
  const controls = node("div", undefined, "matrix-controls");
  const scroll = node("div", undefined, "matrix-scroll");
  const evidencePanel = node("section", undefined, "matrix-evidence");
  let selected;
  evidencePanel.hidden = true;
  scroll.tabIndex = 0;
  scroll.setAttribute("role", "region");
  scroll.setAttribute("aria-label", "Scrollable source and decision table");
  panel.append(controls, scroll);
  list.append(
    node("p", "Cells show linked decisions and individual replay coverage. Stronger color marks higher counts.", "muted"),
    panel, evidencePanel
  );

  function draw() {
    selected = undefined;
    evidencePanel.hidden = true;
    evidencePanel.replaceChildren();
    controls.replaceChildren();
    for (const dimension of dimensions) {
      const { name, items, size, start } = dimension;
      if (items.length <= size) continue;
      const pager = node("div", undefined, "matrix-pager");
      const range = name + " " + (start + 1) + " to " + Math.min(start + size, items.length) + " of " + items.length;
      const counter = node("small", range);
      counter.setAttribute("aria-live", "polite");
      pager.append(counter);
      for (const [label, step] of [["Previous", -size], ["Next", size]]) {
        const button = node("button", label);
        const ariaLabel = label + " " + name.toLowerCase();
        button.setAttribute("aria-label", ariaLabel);
        button.disabled = start + step < 0 || start + step >= items.length;
        button.addEventListener("click", () => {
          dimension.start += step;
          draw();
          const next = [...controls.querySelectorAll("button")].find((candidate) => candidate.getAttribute("aria-label") === ariaLabel);
          (next && !next.disabled ? next : scroll).focus();
        });
        pager.append(button);
      }
      controls.append(pager);
    }
    const [sources, decisions] = dimensions.map(({ items, size, start }) => items.slice(start, start + size));
    const table = node("table", undefined, "matrix-table");
    const caption = node("caption", "Context sources and decisions");
    if (trace) caption.append(node("small", trace, "muted"));
    table.append(caption);
    const headers = table.createTHead().insertRow();
    const corner = node("th", "Context source");
    corner.scope = "col";
    headers.append(corner);
    for (const decision of decisions) {
      const header = node("th", decision.display);
      header.scope = "col";
      header.title = decision.label;
      header.setAttribute("aria-label", decision.label + " · " + decision.trace);
      const qualifier = [decision.kind, !trace && decision.trace, decisionNames.get(decision.display) > 1 && decision.namespace];
      header.append(node("small", qualifier.filter(Boolean).join(" · "), "muted"));
      headers.append(header);
    }
    const rows = table.createTBody();
    for (const source of sources) {
      const row = rows.insertRow();
      const heading = node("th", source.display);
      heading.scope = "row";
      heading.title = source.label;
      heading.setAttribute("aria-label", source.label + " · " + source.trace);
      const qualifier = [!trace && source.trace, sourceNames.get(source.display) > 1 && source.namespace, source.fingerprint?.slice(0, 8)]
        .filter(Boolean).join(" · ");
      if (qualifier) heading.append(node("small", qualifier, "muted"));
      row.append(heading);
      for (const decision of decisions) {
        const slot = row.insertCell();
        const cell = source.cells.get(decision.id);
        if (!cell) {
          slot.textContent = "—";
          continue;
        }
        const { total, measured, ready, gaps } = contextReplayCoverage(cell.occurrences);
        const button = node("button", undefined, "matrix-cell");
        button.dataset.level = String(Math.min(4, 1 + Math.floor(Math.log2(cell.decisions))));
        button.setAttribute("aria-pressed", "false");
        const decisionCount = cell.decisions + (cell.decisions === 1 ? " linked decision" : " linked decisions");
        button.setAttribute("aria-label", source.label + " to " + decision.label + ", " + decisionCount
          + ", " + measured + " of " + total + " contexts measured, " + ready + " ready to plan, " + gaps + " with gaps");
        button.append(node("strong", String(cell.decisions)), node("small", measured + "/" + total + " measured"));
        if (status === "ready") button.append(node("small", ready + " ready to plan"));
        if (status === "gaps") button.append(node("small", gaps + " with gaps"));
        button.addEventListener("click", () => {
          selected?.setAttribute("aria-pressed", "false");
          selected = button;
          selected.setAttribute("aria-pressed", "true");
          const title = node("h2", source.display + " → " + decision.display);
          title.title = source.label + " → " + decision.label;
          title.tabIndex = -1;
          const related = cell.entries.map((entry) => contextPatternCard(entry, status, false));
          const back = node("button", "Back to selected cell");
          back.addEventListener("click", () => button.focus());
          const header = node("div", undefined, "matrix-evidence-heading");
          header.append(title, back);
          evidencePanel.replaceChildren(header, ...related);
          evidencePanel.hidden = false;
          title.focus();
        });
        slot.append(button);
      }
    }
    scroll.replaceChildren(table);
    if (!table.querySelector(".matrix-cell")) {
      scroll.append(node("p", "No links between the sources and decisions on this page.", "muted"));
    }
    scroll.scrollTo(0, 0);
  }
  draw();
}
