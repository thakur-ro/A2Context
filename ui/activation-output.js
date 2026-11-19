function activationFragmentLabel(text) {
  return text ? text.replaceAll(" ", "␠").replaceAll("\n", "↵").replaceAll("\t", "⇥") : "Partial character";
}

function activationTokenTable(fragments, indices, selected, title, selectToken) {
  const table = node("table", undefined, "activation-matrix");
  const head = table.createTHead().insertRow();
  const label = node("th", title);
  label.scope = "col";
  head.append(label);
  for (const index of indices) {
    const header = node("th");
    header.scope = "col";
    const button = node("button", activationFragmentLabel(fragments[index]));
    button.type = "button";
    button.setAttribute("aria-label", "Token " + (index + 1) + " " + button.textContent);
    button.setAttribute("aria-pressed", String(index === selected));
    button.addEventListener("click", () => selectToken(index));
    header.append(button);
    head.append(header);
  }
  return table;
}

function renderActivationOutput(report, call, blockPicker) {
  const scores = report.output_scores;
  const count = scores.fragments.length;
  const details = node("details", undefined, "activation-output");
  details.append(node("summary", "All " + count + " output tokens"));
  details.append(node("p", "Each cell shows the change in log probability of the recorded token after a source swap. Earlier output tokens stay fixed.", "muted"));
  const selection = node("label", undefined, "activation-picker");
  const select = node("select");
  selection.append(node("span", "Output token"), select);
  scores.fragments.forEach((text, index) =>
    select.append(new Option((index + 1) + " · " + activationFragmentLabel(text), index)));
  details.append(selection);
  const scroller = node("div", undefined, "activation-scroll");
  scroller.tabIndex = 0;
  scroller.setAttribute("role", "region");
  scroller.setAttribute("aria-label", "Source effects on recorded tokens");
  const detail = node("div", undefined, "activation-output-detail");
  details.append(scroller, detail);
  const pageSize = 12;
  const format = new Intl.NumberFormat(undefined, { maximumFractionDigits: 4, signDisplay: "exceptZero" });
  let drawnBlock;
  let drawnToken;

  function draw() {
    if (!details.open) return;
    const block = Number(blockPicker.value);
    const selected = Number(select.value);
    if (drawnBlock === block && drawnToken === selected) return;
    drawnBlock = block;
    drawnToken = selected;
    const start = Math.floor(selected / pageSize) * pageSize;
    const indices = Array.from({ length: Math.min(pageSize, count - start) }, (_, index) => start + index);
    const table = activationTokenTable(scores.fragments, indices, selected, "Source swaps at block " + block, (index) => {
      select.value = String(index);
      draw();
      select.focus();
    });
    const body = table.createTBody();
    const scale = report.sources.reduce((maximum, source) => {
      const logProbs = scores.sources[source.context_id].block_log_probs[block];
      return logProbs.reduce((largest, value, index) =>
        Math.max(largest, Math.abs(value - scores.baseline_log_probs[index])), maximum);
    }, 1);
    const original = scores.baseline_log_probs[selected];
    const readout = node("ul", undefined, "activation-readout");
    for (const source of report.sources) {
      const row = body.insertRow();
      const label = node("th");
      label.scope = "row";
      label.append(activationSourceLink(report, call, source));
      row.append(label);
      const series = scores.sources[source.context_id];
      for (const index of indices) {
        const cell = row.insertCell();
        paintActivationDelta(cell, series.block_log_probs[block][index] - scores.baseline_log_probs[index], scale);
        cell.classList.toggle("activation-selected", index === selected);
      }
      const name = call.context.find(({ id }) => id === source.context_id).source_id || source.context_id.replaceAll("-", " ");
      readout.append(node("li", name + " · " + format.format(series.edited_log_probs[selected])
        + " with edited prompt · " + format.format(series.block_log_probs[block][selected]) + " after source swap"));
    }
    scroller.replaceChildren(table);
    detail.replaceChildren(node("h4", "Output token " + (selected + 1) + " of " + count
      + " · " + activationFragmentLabel(scores.fragments[selected])),
      node("p", "Original log probability " + format.format(original), "muted"), readout);
    detail.append(evidenceLinkButton({
      activation_id: report.id, output_token: selected, block,
    }, "this output token"));
  }
  details.addEventListener("toggle", draw);
  blockPicker.addEventListener("change", draw);
  select.addEventListener("change", draw);
  if (!report.checkpoints.length) {
    details.open = true;
    draw();
  }
  return details;
}

function renderVisibilityTokenMatrix(report, token, sourceButton, scale, paired, format) {
  const section = node("section", undefined, "visibility-token-matrix");
  section.append(node("h4", report.baseline_tool_call ? "Across the tool proposal" : "Across the answer"));
  const mode = node("select");
  mode.dataset.visibilityMatrixMode = "";
  mode.append(new Option("Single-source effects", "sources"), new Option("Pair interactions", "pairs"));
  const label = node("label", undefined, "activation-picker");
  label.append(node("span", "Measure"), mode);
  mode.value = paired ? "pairs" : "sources";
  if (paired) section.append(label);
  const metric = node("p", undefined, "muted");
  const scroller = node("div", undefined, "activation-scroll");
  scroller.tabIndex = 0;
  scroller.setAttribute("role", "region");
  scroller.setAttribute("aria-label", "Measured effects across output tokens");
  const page = node("div", undefined, "visibility-token-page");
  const previous = node("button", "Previous tokens");
  const next = node("button", "Next tokens");
  const range = node("span", undefined, "muted");
  range.setAttribute("role", "status");
  previous.type = next.type = "button";
  page.append(previous, range, next);
  const size = 8;
  const count = report.fragments.length;
  page.hidden = count <= size;
  section.append(metric, scroller, page);

  const sources = report.sources.filter(({ masked_log_probs }) => masked_log_probs);
  function selectOutput(index) {
    token.value = String(index);
    token.dispatchEvent(new Event("change"));
  }

  function draw() {
    const showPairs = mode.value === "pairs";
    const selected = Number(token.value);
    const start = selected - selected % size;
    const stop = Math.min(start + size, count);
    const indices = Array.from({ length: stop - start }, (_, index) => start + index);
    metric.textContent = showPairs
      ? "Joint change minus the two individual changes, in log probability units."
      : "Change in the recorded token’s log probability when each source is masked.";
    range.textContent = "Tokens " + (start + 1) + "–" + stop + " of " + count;
    previous.disabled = start === 0;
    next.disabled = stop === count;
    previous.onclick = () => selectPage(start - size, previous);
    next.onclick = () => selectPage(stop, next);
    const table = activationTokenTable(report.fragments, indices, selected, showPairs ? "Source pair" : "Source", (index) => {
      selectOutput(index);
      scroller.querySelector('[aria-pressed="true"]').focus({ preventScroll: true });
    });
    const body = table.createTBody();
    const rows = showPairs ? report.pairs : sources;
    rows.forEach((item, rowIndex) => {
      const row = body.insertRow();
      const heading = node("th");
      heading.scope = "row";
      if (showPairs) {
        const pair = node("button", paired.select.options[rowIndex].textContent);
        pair.type = "button";
        pair.setAttribute("aria-pressed", String(rowIndex === Number(paired.select.value)));
        pair.addEventListener("click", () => {
          paired.select.value = String(rowIndex);
          paired.select.dispatchEvent(new Event("change"));
          paired.select.focus();
        });
        heading.append(pair);
      } else {
        heading.append(sourceButton(item.context_id));
      }
      row.append(heading);
      for (const index of indices) {
        const cell = row.insertCell();
        const delta = showPairs ? paired.effects(item, index)[3] : item.masked_log_probs[index] - report.baseline_log_probs[index];
        paintActivationDelta(cell, delta, showPairs ? paired.scale : scale, format);
        cell.classList.toggle("activation-selected", index === selected);
      }
    });
    const scroll = scroller.scrollLeft;
    scroller.replaceChildren(table);
    scroller.scrollLeft = scroll;
    requestAnimationFrame(() => {
      if (!table.isConnected || !scroller.clientWidth) return;
      const header = table.tHead.rows[0];
      const selectedBounds = header.cells[selected - start + 1].getBoundingClientRect();
      const left = header.cells[0].getBoundingClientRect().right;
      const right = scroller.getBoundingClientRect().right;
      if (selectedBounds.right > right) scroller.scrollLeft += selectedBounds.right - right + 3;
      else if (selectedBounds.left < left) scroller.scrollLeft -= left - selectedBounds.left + 3;
    });
  }

  function selectPage(index, button) {
    selectOutput(index);
    if (button.disabled) scroller.querySelector('thead [aria-pressed="true"]').focus();
  }

  mode.addEventListener("change", draw);
  token.addEventListener("change", draw);
  paired?.select.addEventListener("change", draw);
  draw();
  return { section, mode };
}
