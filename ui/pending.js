"use strict";

function setupPendingView() {
  const form = element("pending-form");
  const input = element("pending-request");
  const status = element("pending-status");
  const output = element("pending-result");
  let active;

  function clearResult() {
    active?.abort();
    output.replaceChildren();
    output.hidden = true;
    status.textContent = "";
  }

  document.querySelector("[data-view='pending']").addEventListener("click", () => {
    linkNotice();
    state.view = "pending";
    render();
  });
  form.addEventListener("input", clearResult);
  element("pending-example").addEventListener("click", async () => {
    clearResult();
    const controller = new AbortController();
    active = controller;
    try {
      const response = await fetch("/examples/pending-returns.json", { signal: controller.signal });
      if (!response.ok) throw new Error("The example could not load.");
      const example = await response.text();
      if (controller.signal.aborted) return;
      form.reset();
      input.value = example;
      form.requestSubmit();
    } catch (error) {
      if (!controller.signal.aborted) status.textContent = error.message;
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    clearResult();
    const controller = new AbortController();
    active = controller;
    status.textContent = "Inspecting request…";
    try {
      const fields = ['"request":' + pendingJson(input.value, "Pending request")];
      for (const name of ["screening", "candidate", "shadow"]) {
        const file = element("pending-" + name).files[0];
        if (!file) continue;
        if (file.size > 1024 * 1024) throw new Error("Choose a report under 1 MiB.");
        fields.push(JSON.stringify(name) + ":" + pendingJson(await file.text(), "Report " + file.name));
      }
      const response = await fetch("/api/pending", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: "{" + fields.join(",") + "}",
        signal: controller.signal,
      });
      const result = await response.json();
      if (!response.ok) throw new Error(result.message);
      if (controller.signal.aborted) return;
      renderPendingEvidence(output, result);
      output.hidden = false;
      status.textContent = "Inspected " + countLabel(result.context.length, "captured input") + ".";
    } catch (error) {
      if (!controller.signal.aborted) status.textContent = error.message;
    }
  });
}

function pendingJson(text, label) {
  try {
    const value = JSON.parse(text);
    if (value && typeof value === "object" && !Array.isArray(value)) return text;
  } catch {
    // Keep the error tied to the field so the submitted contents stay private.
  }
  throw new Error(label + " must be a JSON object.");
}

function renderPendingEvidence(parent, result) {
  parent.append(node("h2", "Request evidence"));
  if (result.model) parent.append(node("p", result.model, "muted"));
  const call = { context: result.context };
  renderPreflightCheck(parent, { status: "matched", report: result.preflight, scope: result.scope }, call, true);
  renderContextScreenReport(parent, result.screening, call, true);
  renderCandidateProbes(parent, result.candidate_probes, call, true);
  renderShadowProbes(parent, result.shadow_probes, call, true);
  const downloads = node("div", undefined, "toolbar");
  for (const [label, filename, attachment] of [
    ["Download preflight receipt", "preflight-receipt", result.preflight.receipt],
    ["Download context screen", "context-screen", result.screening?.report],
  ]) {
    if (!attachment) continue;
    const button = node("button", label);
    button.type = "button";
    button.addEventListener("click", () => saveReplayFile(filename, JSON.stringify(attachment, null, 2)));
    downloads.append(button);
  }
  if (downloads.childElementCount) parent.append(downloads);
  const requestFields = new Map(result.request_fields?.map(({ context_id, fields }) => [context_id, fields]));
  evidenceSection(parent, countLabel(result.context.length, "captured input"), false, (section) => {
    if (!result.request_fields) section.append(node("p", "Supply the original request body to locate editable fields.", "muted"));
    for (const item of result.context) {
      const detail = contextDetails(item, false);
      detail.append(node("small", "Context ID " + item.id, "anchor"));
      for (const field of requestFields.get(item.id) || []) {
        detail.append(node("small", "Matching request path " + requestFieldLabel(field.pointer, field.text_range), "anchor"));
      }
      section.append(detail);
    }
  });
}
