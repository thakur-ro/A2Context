function firstChangedOutputToken(baseline, observed) {
  const common = Math.min(baseline.length, observed.length);
  for (let index = 0; index < common; index++) {
    if (baseline[index] !== observed[index]) return index;
  }
  return baseline.length === observed.length ? null : common;
}

function renderGenerationBranches(report, call) {
  const details = node("details", undefined, "activation-branches");
  details.append(node("summary", "Decoding after a source swap"));
  details.append(node("p", "Each run holds the earlier output tokens fixed, then lets the model generate the rest.", "muted"));
  const field = node("label", undefined, "activation-picker");
  const select = node("select");
  field.append(node("span", "Intervention"), select);
  const choices = report.generation_branches.map((branch, index) => {
    const source = report.sources.find(({ context_id }) => context_id === branch.source_context_id);
    const entry = call.context.find(({ id }) => id === source.context_id);
    const name = entry.source_id || entry.id.replaceAll("-", " ");
    const title = name + " · block " + branch.layer + " · before token " + (branch.start_output_token + 1);
    const option = new Option(title, index);
    option.dataset.block = String(branch.layer);
    select.append(option);
    return { branch, source };
  });
  const detail = node("div", undefined, "activation-branch-detail");
  details.append(field, detail);
  let drawn;
  function draw() {
    const index = Number(select.value);
    if (!details.open || drawn === index) return;
    drawn = index;
    const { branch, source } = choices[index];
    const start = branch.start_output_token;
    detail.replaceChildren(node("h4", "Continuation from token " + (start + 1)));
    detail.append(activationSourceLink(report, call, source));
    const fragments = report.output_scores?.fragments;
    if (fragments && start) {
      detail.append(node("p", "Fixed earlier output", "muted"), node("pre", fragments.slice(0, start).join("")));
    }
    const outputs = node("div", undefined, "activation-branch-grid");
    for (const [title, output] of [
      ["Recorded output", branch.clean],
      ["After activation swap", branch.patched],
      ["After prompt edit", branch.edited_prompt],
    ]) {
      const card = node("section", undefined, "activation-branch-result");
      card.append(node("h5", title));
      const changed = firstChangedOutputToken(report.output_token_ids, output.token_ids);
      const status = output === branch.clean ? "Verified by the clean swap"
        : changed === null ? "Matches recorded tokens" : "First change at token " + (changed + 1);
      card.append(node("p", status + (output.stop_reason === "token_limit" ? " · Token limit reached" : ""), "muted"));
      card.append(node("pre", output.text));
      outputs.append(card);
    }
    detail.append(outputs, node("small", "The tested swap was applied on "
      + countLabel(branch.patched_steps, "decoding step") + ".", "muted"));
    detail.append(evidenceLinkButton({
      activation_id: report.id, branch_index: index, block: branch.layer,
    }, "this generation branch"));
  }
  details.addEventListener("toggle", draw);
  select.addEventListener("change", draw);
  return details;
}
