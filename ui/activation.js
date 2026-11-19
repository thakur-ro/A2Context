const activationMargin = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 2, signDisplay: "exceptZero",
});

function activationReports(run, call, decision) {
  return (data.activation_reports || []).filter(({ location }) => location.run_id === run.id
    && location.call_id === call.id && location.decision_id === decision?.id);
}

function renderActivationReports(parent, reports, call) {
  for (const report of reports) {
    const panel = node("section", undefined, "activation-report");
    panel.dataset.activationId = report.id;
    if (report.kind === "source_visibility") {
      renderSourceVisibility(panel, report, call);
      parent.append(panel);
      continue;
    }
    if (report.version === 2) {
      renderActivationTimeline(panel, report, call);
      parent.append(panel);
      continue;
    }
    const [first, second] = report.alternatives;
    panel.append(node("h3", first + " or " + second + " at the next token"));
    panel.append(node("p", "Positive log odds favor " + first + ". Negative log odds favor " + second + ".", "muted"));
    const summary = node("dl", undefined, "activation-outcomes");
    const formatOutcome = (outcome) => outcome.answer + " · " + activationMargin.format(outcome.margin) + " log odds";
    summary.append(node("dt", "Original prompt"), node("dd", formatOutcome(report.baseline)));
    const labels = new Map();
    for (const source of report.sources) {
      const label = activationEditLabel(source);
      labels.set(source.context_id, label);
      const term = node("dt");
      term.append(activationSourceLink(report, call, source));
      summary.append(term, node("dd", formatOutcome(source.edited_outcome)));
    }
    panel.append(summary);
    panel.append(node("p", "Each cell shows the change from the original log odds after one decoder-block swap. Columns follow model depth at a single prediction position.", "muted"));
    const picker = activationBlockPicker(report.layer_count);
    const sweep = activationSweep(report.layer_count, report.sweeps, report.baseline.margin, labels);
    picker.select.addEventListener("change", () => sweep.showBlock(Number(picker.select.value)));
    sweep.showBlock(0);
    panel.append(sweep.scroller, picker.label, sweep.readout);
    const provenance = activationProvenance(report);
    provenance.append(node("p", "Zero-based prediction position " + report.prediction_position
      + ". Compared token IDs " + report.choice_token_ids.join(" and ") + ".", "muted"));
    provenance.append(node("p", "Block " + report.checks.layer + " controls reproduced the original and edited scores within 0.05 log odds.", "muted"));
    panel.append(provenance);
    parent.append(panel);
  }
}

function visibilityBoundaryNote({ before, after }, label) {
  const parts = [
    before && before.length + " before",
    after && after.length + " after",
  ].filter(Boolean);
  const prefix = label ? label + " · " : "";
  const note = node("small", prefix + "Extra whitespace bytes in mask · " + parts.join(" and "), "anchor");
  note.title = JSON.stringify({ before, after });
  return note;
}

function renderSourceVisibility(panel, report, call) {
  const precision = visibilityEffectFormat;
  panel.append(node("h3", "Context visibility test"));
  panel.append(node("p", "Earlier output tokens stay fixed. A negative source effect means the recorded token became less likely.", "muted"));
  const label = node("label", undefined, "activation-picker");
  const select = node("select");
  select.dataset.visibilityToken = "";
  label.append(node("span", "Output token"), select);
  report.fragments.forEach((text, index) => select.append(new Option(
    (index + 1) + " · " + activationFragmentLabel(text), index)));
  let strongest = 0;
  let scale = 0;
  for (const source of report.sources) {
    source.masked_log_probs?.forEach((value, index) => {
      const absolute = Math.abs(value - report.baseline_log_probs[index]);
      if (absolute > scale) {
        strongest = index;
        scale = absolute;
      }
    });
  }
  const table = node("table", undefined, "activation-matrix activation-visibility");
  const headings = table.createTHead().insertRow();
  for (const name of ["Captured source", "Change in log probability"]) {
    const heading = node("th", name);
    heading.scope = "col";
    headings.append(heading);
  }
  const body = table.createTBody();
  const reasons = {
    text_not_captured: "Text unavailable", no_request_fragment: "Request position unknown",
    unsupported_request_field: "Unsupported request field", rendered_source_unaligned: "Rendered text did not align",
    request_object_not_found: "No matching request object", request_object_ambiguous: "Multiple matching request objects",
    token_boundary_unaligned: "Token boundaries did not align",
  };
  const names = new Map(call.context.map((item) => [item.id, item.source_id || item.id.replaceAll("-", " ")]));
  const sourceButton = (id) => occurrenceButton(report.location, names.get(id), { context_item_id: id });
  const reference = { activation_id: report.id };
  const paired = report.pairs?.length ? sourceVisibilityPairs(report, names, sourceButton, precision) : null;
  const targets = node("div", undefined, "decision-actions");
  select.value = String(paired?.startToken ?? strongest);
  const measured = report.sources.filter((source) => source.masked_log_probs).length;
  const matrix = measured ? renderVisibilityTokenMatrix(report, select, sourceButton, scale, paired, precision) : null;
  function selectMeasure() {
    if (!paired) return;
    paired.panel.hidden = matrix.mode.value !== "pairs";
    reference.visibility_pair = paired.panel.hidden ? undefined : Number(paired.select.value);
  }
  matrix?.mode.addEventListener("change", selectMeasure);
  paired?.select.addEventListener("change", selectMeasure);
  function draw() {
    const index = Number(select.value);
    reference.visibility_token = index;
    const activeTargets = (report.targets || []).filter(({ token_range: [start, end] }) => start <= index && index < end);
    targets.replaceChildren(...activeTargets.map(({ key }) => {
      const button = node("button", "Find " + key);
      button.type = "button";
      button.addEventListener("click", () => openVisibilityPatterns(report, index,
        reference.visibility_pair === undefined ? "source" : "pair", key));
      return button;
    }));
    targets.hidden = !activeTargets.length;
    const baseline = report.baseline_log_probs[index];
    const rows = report.sources.map((source) => ({
      source, delta: source.masked_log_probs ? source.masked_log_probs[index] - baseline : null,
    }));
    rows.sort((a, b) => Math.abs(b.delta ?? 0) - Math.abs(a.delta ?? 0) || Number(a.delta === null) - Number(b.delta === null));
    body.replaceChildren();
    for (const { source, delta } of rows) {
      const row = body.insertRow();
      const heading = node("th");
      heading.scope = "row";
      const button = sourceButton(source.context_id);
      if (delta !== null) button.dataset.visibilitySource = source.context_id;
      heading.append(button);
      if (source.request_pointer) heading.append(node("small", source.request_pointer, "anchor"));
      if (source.boundary_whitespace) heading.append(visibilityBoundaryNote(source.boundary_whitespace));
      row.append(heading);
      const effect = row.insertCell();
      if (delta === null) effect.textContent = reasons[source.unavailable];
      else paintActivationDelta(effect, delta, scale, precision);
    }
    paired?.draw(index);
    selectMeasure();
  }
  select.addEventListener("change", draw);
  draw();
  const compare = node("button", "Find recurring effects");
  compare.type = "button";
  compare.addEventListener("click", () => openVisibilityPatterns(report, Number(select.value),
    reference.visibility_pair === undefined ? "source" : "pair"));
  const provenance = activationProvenance(report);
  provenance.append(node("p", countLabel(measured, "measured source")
    + " of " + report.sources.length + ". The all-visible control reproduced the original scores within 0.00001 log units.", "muted"));
  if (matrix) panel.append(matrix.section);
  panel.append(label);
  if (paired) panel.append(paired.panel);
  const actions = node("div", undefined, "decision-actions");
  actions.append(compare, evidenceLinkButton(reference, "this visibility measurement"));
  panel.append(table, actions, targets, provenance);
}

function sourceVisibilityPairs(report, names, sourceButton, precision) {
  const panel = node("section", undefined, "activation-visibility-pairs");
  const picker = node("label", undefined, "activation-picker");
  const select = node("select");
  select.dataset.visibilityPair = "";
  const scores = new Map(report.sources.map(({ context_id, masked_log_probs }) => [context_id, masked_log_probs]));
  let startToken = 0;
  let strongest = -1;
  let startPair = 0;
  function effects(pair, index) {
    const [first, second] = pair.context_ids.map((id) => scores.get(id)[index]);
    const baseline = report.baseline_log_probs[index];
    const joint = pair.masked_log_probs[index];
    return [first - baseline, second - baseline, joint - baseline, visibilityInteraction(baseline, first, second, joint)];
  }
  report.pairs.forEach((pair, pairIndex) => {
    select.append(new Option(pair.context_ids.map((id) => names.get(id)).join(" + "), pairIndex));
    pair.masked_log_probs.forEach((_, index) => {
      const interaction = Math.abs(effects(pair, index)[3]);
      if (interaction > strongest) {
        strongest = interaction;
        startToken = index;
        startPair = pairIndex;
      }
    });
  });
  select.value = String(startPair);
  picker.append(node("span", "Source pair"), select);
  const detail = node("div", undefined, "activation-visibility-pair-detail");
  panel.append(node("h4", "Paired masks"), picker, detail);
  let outputToken = startToken;
  function draw(index) {
    outputToken = index;
    const pair = report.pairs[Number(select.value)];
    const [first, second] = pair.context_ids;
    const values = effects(pair, index);
    const labels = ["Only " + names.get(first), "Only " + names.get(second), "Both masked", "Interaction"];
    const metrics = node("dl", undefined, "activation-visibility-pair-scores");
    labels.forEach((label, i) => {
      const item = node("div");
      item.append(node("dt", label), node("dd", precision.format(values[i])));
      metrics.append(item);
    });
    const links = node("div", undefined, "activation-visibility-pair-links");
    links.append(sourceButton(first), sourceButton(second));
    detail.replaceChildren(metrics, links);
  }
  select.addEventListener("change", () => draw(outputToken));
  return { panel, select, draw, startToken, effects, scale: strongest };
}

function openActivationCheckpoint(reference) {
  const panel = [...element("decision-activations").querySelectorAll("[data-activation-id]")]
    .find((item) => item.dataset.activationId === reference.activation_id);
  if (reference.visibility_token !== undefined) {
    const token = panel?.querySelector("[data-visibility-token]");
    const pair = panel?.querySelector("[data-visibility-pair]");
    if (!token?.options[reference.visibility_token]
      || (reference.visibility_pair !== undefined && !pair?.options[reference.visibility_pair])) return;
    token.value = String(reference.visibility_token);
    if (reference.visibility_pair !== undefined) pair.value = String(reference.visibility_pair);
    const mode = panel.querySelector("[data-visibility-matrix-mode]");
    if (mode) mode.value = reference.visibility_pair === undefined ? "sources" : "pairs";
    token.dispatchEvent(new Event("change"));
    if (reference.visibility_source !== undefined) {
      const source = [...panel.querySelectorAll("[data-visibility-source]")]
        .find((button) => button.dataset.visibilitySource === reference.visibility_source);
      source?.closest("tr").cells[1].classList.add("activation-selected");
      return source;
    }
    return reference.visibility_pair === undefined ? token : pair;
  }
  const picker = panel?.querySelector(".activation-picker select");
  if (!picker?.options[reference.block]) return;
  if (reference.branch_index !== undefined) {
    const branches = panel.querySelector(".activation-branches");
    const select = branches?.querySelector("select");
    if (select?.options[reference.branch_index]?.dataset.block !== String(reference.block)) return;
    branches.open = true;
    picker.value = String(reference.block);
    picker.dispatchEvent(new Event("change"));
    select.value = String(reference.branch_index);
    select.dispatchEvent(new Event("change"));
    return branches.querySelector(".activation-branch-detail h4");
  }
  if (reference.output_token !== undefined) {
    const output = panel.querySelector(".activation-output");
    const select = output?.querySelector("select");
    if (!select?.options[reference.output_token]) return;
    output.open = true;
    picker.value = String(reference.block);
    select.value = String(reference.output_token);
    picker.dispatchEvent(new Event("change"));
    return output.querySelector(".activation-output-detail h4");
  }
  const checkpoints = panel.querySelectorAll(".activation-timeline thead button");
  if (!checkpoints[reference.checkpoint_index]) return;
  picker.value = String(reference.block);
  checkpoints[reference.checkpoint_index].click();
  picker.dispatchEvent(new Event("change"));
  return panel.querySelector(".activation-checkpoint h4");
}

function activationEditLabel(source) {
  return source.before + " → " + source.after;
}

function activationSourceLink(report, call, source) {
  const item = call.context.find(({ id }) => id === source.context_id);
  const name = item.source_id || item.id.replaceAll("-", " ");
  const edit = activationEditLabel(source);
  const link = occurrenceButton(report.location, name + " · " + edit, { context_item_id: source.context_id });
  link.setAttribute("aria-label", "Open context for " + name + " edit " + edit);
  return link;
}

function activationProvenance(report) {
  const details = node("details", undefined, "activation-provenance");
  details.append(node("summary", "Probe details"));
  details.append(node("p", report.model_id + " · " + report.runtime));
  details.append(node("p", "Model revision " + report.model_revision, "muted"));
  return details;
}

function activationBlockPicker(layerCount) {
  const label = node("label", undefined, "activation-picker");
  const select = node("select");
  label.append(node("span", "Inspect block"), select);
  for (let block = 0; block < layerCount; block++) select.append(node("option", String(block)));
  return { label, select };
}

function renderActivationTimeline(panel, report, call) {
  const toolCalls = report.tool_calls;
  panel.append(node("h3", toolCalls ? "Context influence across the tool proposal" : "Context influence across the answer"));
  if (report.checkpoints.length) {
    panel.append(node("p", "Earlier generated tokens stay fixed at each checkpoint.", "muted"));
  }
  const picker = activationBlockPicker(report.layer_count);
  panel.append(picker.label);
  if (report.output_scores) panel.append(renderActivationOutput(report, call, picker.select));
  if (report.generation_branches) panel.append(renderGenerationBranches(report, call));
  if (report.checkpoints.length) renderActivationContrasts(panel, report, call, picker.select);
  const outcomes = node("details", undefined, "activation-provenance");
  const baseline = toolCalls ? activationProposalText(toolCalls.baseline) : report.baseline_answer;
  outcomes.append(node("summary", toolCalls
    ? "Tool proposals from the original and edited prompts" : "Generated answers from the original and edited prompts"));
  outcomes.append(node("p", "Original prompt"), node("pre", baseline));
  for (const source of report.sources) {
    const proposal = toolCalls?.edited[source.context_id];
    outcomes.append(activationSourceLink(report, call, source),
      node("pre", proposal ? activationProposalText(proposal) : source.edited_outcome));
  }
  panel.append(outcomes, activationProvenance(report));
}

function renderActivationContrasts(panel, report, call, blockPicker) {
  const scroller = node("div", undefined, "activation-scroll");
  scroller.tabIndex = 0;
  scroller.setAttribute("role", "region");
  scroller.setAttribute("aria-label", "Source effects across output tokens");
  const table = node("table", undefined, "activation-matrix activation-timeline");
  const header = table.createTHead().insertRow();
  const heading = node("th", "Source swap and change in log odds");
  heading.scope = "col";
  header.append(heading);
  const tabs = report.checkpoints.map((step, index) => {
    const cell = node("th");
    cell.scope = "col";
    const button = node("button", step.label + " · token " + (step.output_token + 1));
    button.type = "button";
    button.addEventListener("click", () => selectStep(index));
    cell.append(button);
    cell.append(node("small", step.alternatives.join(" vs. "), "muted"));
    header.append(cell);
    return button;
  });
  const body = table.createTBody();
  const labels = new Map(report.sources.map((source) => [source.context_id, activationEditLabel(source)]));
  const rows = report.sources.map((source) => {
    const row = body.insertRow();
    const heading = node("th");
    heading.scope = "row";
    heading.append(activationSourceLink(report, call, source));
    row.append(heading);
    return report.checkpoints.map((step) => ({
      cell: row.insertCell(),
      sweep: step.sweeps.find(({ source_context_id, target }) => source_context_id === source.context_id && target === "source_tokens"),
      baseline: step.baseline_margin,
    }));
  });
  scroller.append(table);
  panel.append(scroller, node("p", "Select a checkpoint to inspect the fixed text, original and edited scores, and the full block sweep.", "muted"));
  const detail = node("div", undefined, "activation-checkpoint");
  let activeSweep;
  function selectStep(index) {
    const step = report.checkpoints[index];
    tabs.forEach((button, position) => button.setAttribute("aria-pressed", String(position === index)));
    const [base, alternative] = step.alternatives;
    detail.replaceChildren(node("h4", step.label + " at token " + (step.output_token + 1) + " of " + report.output_token_ids.length));
    detail.append(node("p", "Positive log odds favor " + base + ". Negative log odds favor " + alternative + ".", "muted"));
    detail.append(node("p", "Fixed text before this token", "muted"));
    detail.append(node("pre", step.answer_prefix || "Start of the answer"));
    const margins = node("dl", undefined, "activation-outcomes");
    margins.append(node("dt", "Original prompt"), node("dd", activationMargin.format(step.baseline_margin) + " log odds"));
    for (const source of report.sources) {
      const name = call.context.find(({ id }) => id === source.context_id).source_id || source.context_id.replaceAll("-", " ");
      margins.append(node("dt", name + " edit"), node("dd", activationMargin.format(step.edited_margins[source.context_id]) + " log odds"));
    }
    detail.append(margins);
    activeSweep = activationSweep(report.layer_count, step.sweeps, step.baseline_margin, labels);
    activeSweep.showBlock(Number(blockPicker.value));
    detail.append(activeSweep.scroller, activeSweep.readout);
    detail.append(node("small", "Block " + step.checks.layer + " controls reproduced the original and edited scores within 0.05 log odds.", "muted"));
  }
  function updateBlock() {
    const block = Number(blockPicker.value);
    const values = rows.flat().map(({ cell, sweep, baseline }) => ({ cell, delta: sweep.margins[block] - baseline }));
    const scale = Math.max(1, ...values.map(({ delta }) => Math.abs(delta)));
    for (const { cell, delta } of values) paintActivationDelta(cell, delta, scale);
    activeSweep.showBlock(block);
  }
  blockPicker.addEventListener("change", updateBlock);
  selectStep(0);
  updateBlock();
  panel.append(detail);
}

function activationProposalText({ name, arguments: args }) {
  const values = Object.entries(args).map(([key, value]) => key + "=" + JSON.stringify(value));
  return name + "(" + values.join(", ") + ")";
}

function paintActivationDelta(cell, delta, scale, format = activationMargin) {
  cell.textContent = format.format(delta);
  const color = delta > 0 ? "var(--accent)" : "var(--warning)";
  cell.style.background = delta === 0 ? "" :
    "color-mix(in srgb, " + color + " " + (25 * Math.abs(delta) / scale) + "%, var(--surface))";
}

function activationSweep(layerCount, sweeps, baseline, labels) {
  const scroller = node("div", undefined, "activation-scroll");
  scroller.tabIndex = 0;
  scroller.setAttribute("role", "region");
  scroller.setAttribute("aria-label", "Activation effects by decoder block");
  const table = node("table", undefined, "activation-matrix");
  const header = table.createTHead().insertRow();
  const siteHeading = node("th", "Edited run and swap site");
  siteHeading.scope = "col";
  header.append(siteHeading);
  for (let block = 0; block < layerCount; block++) {
    const heading = node("th", String(block));
    heading.scope = "col";
    header.append(heading);
  }
  const scale = Math.max(1, ...sweeps.flatMap(({ margins }) => margins.map((margin) => Math.abs(margin - baseline))));
  const body = table.createTBody();
  const rows = sweeps.map((sweep) => {
    const label = labels.get(sweep.source_context_id)
      + (sweep.target === "prediction_token" ? " at prediction token" : " at source tokens");
    const row = body.insertRow();
    const heading = node("th", label);
    heading.scope = "row";
    row.append(heading);
    for (const margin of sweep.margins) paintActivationDelta(row.insertCell(), margin - baseline, scale);
    return { label, row, sweep };
  });
  scroller.append(table);
  const readout = node("ul", undefined, "activation-readout");
  readout.setAttribute("aria-live", "polite");
  const tableRows = [header, ...rows.map(({ row }) => row)];
  let previous;
  function showBlock(index) {
    for (const row of tableRows) {
      if (previous !== undefined) row.cells[previous + 1].classList.remove("activation-selected");
      row.cells[index + 1].classList.add("activation-selected");
    }
    previous = index;
    readout.replaceChildren(...rows.map(({ label, sweep }) => node("li", label
      + " · token range [" + sweep.token_range.join(", ") + ") · "
      + activationMargin.format(sweep.margins[index]) + " patched log odds")));
  }
  return { scroller, readout, showBlock };
}
