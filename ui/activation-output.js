function activationFragmentLabel(text) {
  return text ? text.replaceAll(" ", "␠").replaceAll("\n", "↵").replaceAll("\t", "⇥") : "Partial character";
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
    const table = node("table", undefined, "activation-matrix");
    const head = table.createTHead().insertRow();
    const title = node("th", "Source swaps at block " + block);
    title.scope = "col";
    head.append(title);
    for (const index of indices) {
      const header = node("th");
      header.scope = "col";
      const button = node("button", activationFragmentLabel(scores.fragments[index]));
      button.type = "button";
      button.setAttribute("aria-label", "Token " + (index + 1) + " " + button.textContent);
      button.setAttribute("aria-pressed", String(index === selected));
      button.addEventListener("click", () => {
        select.value = String(index);
        draw();
        select.focus();
      });
      header.append(button);
      head.append(header);
    }
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
