"use strict";

function parseContainer(text) {
  if (!/^[\s]*[\[{]/.test(text)) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

async function replayJson(endpoint, body) {
  const options = body === undefined ? undefined : {
    method: "POST", headers: { "Content-Type": "application/json" }, body,
  };
  const response = await fetch(endpoint, options);
  const result = await response.json().catch(() => null);
  if (!response.ok || result === null) {
    throw new Error(result?.message || "The local inspector could not process the replay request.");
  }
  return result;
}

function replayCandidate(item, paths, suggested) {
  const card = node("article", undefined, "finding replay-candidate");
  const selectLabel = node("label");
  const checked = node("input");
  checked.type = "checkbox";
  checked.checked = suggested;
  const name = [item.source_id || item.id, item.source_version, item.role].filter(Boolean).join(" · ");
  selectLabel.append(checked, " " + name);
  const details = node("div");
  details.hidden = !suggested;
  checked.addEventListener("change", () => { details.hidden = !checked.checked; });

  const captured = node("details");
  captured.append(node("summary", "Captured text"), node("pre", item.text));
  const pathLabel = node("label", "Request path", "form-field");
  const pointer = node("select");
  if (paths.length > 1) pointer.add(new Option("Choose which matching field to change", ""));
  const fields = new Map(paths.map((field) => [field.pointer, field]));
  for (const { pointer: path, kind, text_range } of paths) {
    pointer.add(new Option(requestFieldLabel(path, text_range) + (kind === "text" ? "" : " · JSON " + kind), path));
  }
  pathLabel.append(pointer);
  const operationLabel = node("label", "Change", "form-field");
  const operation = node("select");
  operation.add(new Option("Remove JSON field or array entry", "remove_json"));
  operation.add(new Option("Replace with JSON", "replace_json"));
  operationLabel.append(operation);
  const replacementLabel = node("label", undefined, "form-field");
  const replacementTitle = node("span");
  const replacement = node("textarea");
  replacement.rows = 2;
  replacementLabel.append(replacementTitle, replacement);
  let previousKind;
  const updateEdit = () => {
    const kind = fields.get(pointer.value)?.kind;
    const structured = kind === "object" || kind === "array";
    operationLabel.hidden = !structured;
    replacementLabel.hidden = !kind || (structured && operation.value === "remove_json");
    replacementTitle.textContent = structured ? "Replacement JSON" : "Replacement text";
    if (kind !== previousKind) {
      replacement.value = structured ? item.text : item.text === "[context omitted]" ? "" : "[context omitted]";
      previousKind = kind;
    }
  };
  pointer.addEventListener("change", updateEdit);
  operation.addEventListener("change", updateEdit);
  updateEdit();
  details.append(captured, pathLabel, operationLabel, replacementLabel);
  card.append(selectLabel, evidence(item.evidence), details);
  return { item, name, card, checked, pointer, replacement, operation, fields };
}

function disjointReplayRanges(first, second) {
  return Boolean(first && second && (first.end_byte <= second.start_byte || second.end_byte <= first.start_byte));
}

function saveReplayFile(name, text) {
  const url = URL.createObjectURL(new Blob([text], { type: "application/json" }));
  const download = node("a");
  download.href = url;
  download.download = name.replace(/[^a-zA-Z0-9._-]/g, "-") + ".json";
  download.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function openReplayPlanner(run, call, decision, suggested = []) {
  openInterventionPlanner({ run, call, decision, suggested });
}

function openShadowPlanner(request, result, suggested = []) {
  openInterventionPlanner({
    call: { context: result.context }, suggested,
    pending: { request, fields: result.request_fields },
  });
}

function openInterventionPlanner({ run, call, decision, suggested, pending }) {
  const dialog = element("replay-planner");
  const form = element("replay-form");
  const input = element("replay-request");
  const list = element("replay-candidates");
  const fieldset = element("replay-interventions");
  const status = element("replay-status");
  const submit = element("download-replay-case");
  const testPair = element("replay-test-pair");
  const controlField = element("replay-control-field");
  const negativeControl = element("replay-negative-control");
  const matchKind = element("replay-match-kind");
  const useRecorded = element("use-recorded-request");
  const downloadEvidence = element("download-replay-evidence");
  const requestSource = element("replay-request-source");
  const caseName = pending ? "shadow" : "replay";
  const hasCapture = !pending && hasRecordedRequest(run.id, call.id);
  const context = call.context.filter((item) => item.availability === "included" && typeof item.text === "string");
  const fieldsEndpoint = !pending && "/api/replay-fields?run=" + encodeURIComponent(run.id) + "&call=" + encodeURIComponent(call.id);
  let requestText = "";
  let candidates = [];
  let revision = 0;
  let caseRevision = 0;

  form.reset();
  element("shadow-proposal").hidden = !pending;
  element("shadow-proposal").disabled = !pending;
  element("replay-planner-title").textContent = pending ? "Prepare shadow probe" : "Prepare controlled replay";
  element("replay-match-label").textContent = pending ? "Proposal match" : "Decision match";
  submit.textContent = pending ? "Download shadow case" : "Download replay case";
  useRecorded.hidden = !hasCapture;
  useRecorded.disabled = false;
  downloadEvidence.hidden = !pending;
  downloadEvidence.textContent = pending ? "Download pending request" : "Download call evidence";
  downloadEvidence.onclick = pending ? () => saveReplayFile("pending-request", pending.request) : null;
  element("replay-manual-request").hidden = hasCapture || Boolean(pending);
  requestSource.textContent = pending
    ? "Run the case with this request and a runner that stops before dispatch."
    : hasCapture
    ? "Use the request captured on this call. Download the call evidence to run the case with the CLI."
    : "Choose the request body captured for this call. Request provenance cannot be verified from this trace.";
  const updateMatchFields = () => {
    element("replay-match-fields").hidden = matchKind.value !== "outcome_fields";
    element("replay-match-text").hidden = matchKind.value !== "text_capture";
  };
  matchKind.onchange = updateMatchFields;
  updateMatchFields();
  element("replay-case-id").value = caseName + "-" + crypto.randomUUID();
  element("replay-target").hidden = Boolean(pending);
  element("replay-target").textContent = pending ? "" : [decision.label, run.conversation_id || run.id, call.id].join(" · ");
  status.textContent = "";
  fieldset.hidden = true;
  list.replaceChildren();
  submit.disabled = true;
  testPair.disabled = true;
  controlField.hidden = true;
  negativeControl.value = "";
  dialog.showModal();
  element("close-replay-planner").onclick = () => dialog.close();
  const closePlanner = () => {
    // A close event can arrive after the next planner has already opened.
    if (dialog.open) return;
    revision += 1;
    caseRevision += 1;
    form.reset();
    list.replaceChildren();
    requestText = "";
    candidates = [];
    downloadEvidence.onclick = null;
  };
  dialog.onclose = closePlanner;
  const isCurrentPlanner = () => dialog.open && dialog.onclose === closePlanner;

  const updateSelection = () => {
    const selected = candidates.filter(({ checked }) => checked.checked);
    submit.disabled = selected.length === 0;
    testPair.disabled = selected.length !== 2;
    if (testPair.disabled) testPair.checked = false;
    controlField.hidden = testPair.disabled;
    const previousControl = negativeControl.value;
    negativeControl.replaceChildren(new Option("No negative control", ""));
    if (selected.length === 2) {
      for (const { item, name } of selected) {
        const label = selected[0].name === selected[1].name ? name + " · " + item.id : name;
        negativeControl.add(new Option(label, item.id));
      }
      if (selected.some(({ item }) => item.id === previousControl)) negativeControl.value = previousControl;
    }
  };
  const cancelValidation = () => {
    caseRevision += 1;
    if (status.textContent === "Checking " + caseName + " case…") status.textContent = "";
  };
  form.oninput = cancelValidation;
  form.onchange = (event) => {
    cancelValidation();
    if (candidates.some(({ checked }) => checked === event.target)) updateSelection();
  };
  const resetRequest = () => {
    cancelValidation();
    requestText = "";
    candidates = [];
    list.replaceChildren();
    fieldset.hidden = true;
    updateSelection();
    status.textContent = "";
    return ++revision;
  };
  const loadRequest = (text, fields) => {
    const paths = new Map(fields.map((item) => [item.context_id, item.fields]));
    candidates = context.filter((item) => paths.has(item.id)).map((item) =>
      replayCandidate(item, paths.get(item.id), suggested.includes(item.id))
    );
    if (!candidates.length) {
      status.textContent = "No captured context matches an exact text value, JSON object or array, or declared fragment in this request.";
      return;
    }
    status.textContent = "";
    requestText = text;
    for (const candidate of candidates) list.append(candidate.card);
    fieldset.hidden = false;
    updateSelection();
  };
  if (pending) {
    loadRequest(pending.request, pending.fields);
    if (suggested.length === 2 && !testPair.disabled) testPair.checked = true;
  }
  input.onchange = async () => {
    const file = input.files[0];
    const current = resetRequest();
    if (!file) return;
    const text = await file.text().catch(() => null);
    if (!isCurrentPlanner() || current !== revision) return;
    if (text === null) {
      status.textContent = "This request file could not be read.";
      return;
    }
    try {
      const plan = await replayJson(fieldsEndpoint, text);
      if (isCurrentPlanner() && current === revision) loadRequest(text, plan.fields);
    } catch (error) {
      if (!isCurrentPlanner() || current !== revision) return;
      status.textContent = error.message;
    }
  };
  useRecorded.onclick = async () => {
    const current = resetRequest();
    useRecorded.disabled = true;
    status.textContent = "Loading the captured request…";
    try {
      const plan = await replayJson(fieldsEndpoint);
      if (!isCurrentPlanner() || current !== revision) return;
      const [selectedRun] = JSON.parse(plan.evidence_json);
      const [selectedCall] = selectedRun.calls;
      if (selectedRun.id !== run.id || selectedCall.id !== call.id) throw new Error("Unexpected call");
      const captured = selectedCall.recorded_request;
      status.textContent = "";
      loadRequest(captured.body, plan.fields);
      requestSource.textContent = "Captured on this call · " + captured.evidence.field;
      downloadEvidence.onclick = () => saveReplayFile("call-evidence-" + call.id, plan.evidence_json);
      downloadEvidence.hidden = false;
    } catch {
      if (!isCurrentPlanner() || current !== revision) return;
      status.textContent = "Could not load the captured request. Try again.";
      useRecorded.disabled = false;
    }
  };

  form.onsubmit = async (event) => {
    event.preventDefault();
    const current = ++caseRevision;
    const id = element("replay-case-id").value.trim();
    const runner = element("replay-runner-id").value.trim();
    const samples = element("replay-samples").valueAsNumber;
    if (!id || !runner || !Number.isSafeInteger(samples) || samples < 1) {
      status.textContent = "Enter a case ID, runner ID, and a positive whole number of trials.";
      return;
    }
    const interventions = [];
    for (const candidate of candidates.filter(({ checked }) => checked.checked)) {
      const { item, pointer, replacement, operation, fields } = candidate;
      const field = fields.get(pointer.value);
      if (!field || interventions.some((edit) => edit.pointer === pointer.value && !disjointReplayRanges(edit.text_range, field.text_range))) {
        status.textContent = "Choose distinct request paths or non-overlapping text ranges for the interventions.";
        return;
      }
      const intervention = { context_id: item.id, pointer: pointer.value, replacement: replacement.value };
      const { kind, text_range } = field;
      if (kind === "text") {
        if (replacement.value === item.text) {
          status.textContent = "Choose different replacement text for " + item.id + ".";
          return;
        }
        if (text_range) {
          intervention.operation = "replace_text_fragment";
          intervention.text_range = text_range;
        }
      } else {
        intervention.operation = operation.value;
        if (operation.value === "remove_json") {
          intervention.replacement = "";
        } else {
          const parsed = parseContainer(replacement.value);
          if (!parsed || Array.isArray(parsed) !== (kind === "array")
             || replacement.value === item.text) {
            status.textContent = "Enter different replacement JSON of the same container type for " + item.id + ".";
            return;
          }
        }
      }
      interventions.push(intervention);
    }
    if (!requestText || !interventions.length) return;
    if (testPair.checked || negativeControl.value) {
      const [left, right] = interventions.map(({ pointer }) => pointer);
      if (left.startsWith(right + "/") || right.startsWith(left + "/")) {
        status.textContent = "Choose non-overlapping request paths for the two edits.";
        return;
      }
    }
    const metadata = { version: 1, id, runner_id: runner, samples, interventions };
    if (!pending) metadata.location = { run_id: run.id, call_id: call.id, decision_id: decision.id };
    if (testPair.checked) metadata.test_pair = true;
    if (negativeControl.value) metadata.negative_control_context_id = negativeControl.value;
    if (element("replay-independent").checked) metadata.sampling_design = "independent";
    if (matchKind.value === "outcome_fields") {
      const pointers = element("replay-match-pointers").value.split("\n").filter((line) => line.trim());
      if (!pointers.length || new Set(pointers).size !== pointers.length || pointers.some((path) => !/^\/(?:[^~]|~[01])*$/.test(path))) {
        status.textContent = "Enter distinct JSON Pointers such as /sku, one per line.";
        return;
      }
      metadata.match_rule = { kind: "outcome_fields", pointers };
    } else if (matchKind.value === "text_capture") {
      const pattern = element("replay-match-pattern").value;
      if (!pattern.trim()) {
        status.textContent = "Enter a regular expression with one capture group.";
        return;
      }
      metadata.match_rule = { kind: "text_capture", pattern };
    } else if (matchKind.value === "kind_and_label") {
      metadata.match_rule = { kind: "kind_and_label" };
    }
    let field = "request";
    let value = requestText;
    if (pending) {
      const kind = element("shadow-kind").value.trim();
      const label = element("shadow-label").value.trim();
      const outcome = element("shadow-outcome").value.trim();
      try {
        JSON.parse(outcome);
      } catch {
        status.textContent = "Enter the expected outcome as JSON.";
        return;
      }
      if (!kind || !label) {
        status.textContent = "Enter the expected proposal kind and label.";
        return;
      }
      field = "expected";
      value = JSON.stringify({ kind, label }).slice(0, -1) + ',"outcome":' + outcome + "}";
    }
    // Insert the raw JSON to preserve numeric values in the request or expected outcome.
    const json = JSON.stringify(metadata, null, 2).slice(0, -2) + ',\n  "' + field + '": ' + value + "\n}\n";
    status.textContent = "Checking " + caseName + " case…";
    try {
      const body = pending ? '{"request":' + requestText + ',"case":' + json + "}" : json;
      await replayJson("/api/" + caseName + "-check", body);
      if (!isCurrentPlanner() || current !== caseRevision) return;
      saveReplayFile(id, json);
      status.textContent = (pending ? "Shadow" : "Replay") + " case downloaded.";
    } catch (error) {
      if (isCurrentPlanner() && current === caseRevision) status.textContent = error.message;
    }
  };
}
