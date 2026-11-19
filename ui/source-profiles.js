"use strict";

let sourceProfileId = "";
let sourceSearchTimer;
const sourceDates = ["after", "before"];
const sourceComparisonDates = ["after_b", "before_b"];
const sourceProfileField = (field) => element("source-profile-" + field);

function queueSourceProfileSearch() {
  window.clearTimeout(sourceSearchTimer);
  sourceProfileId = "";
  element("pattern-list").replaceChildren();
  sourceSearchTimer = window.setTimeout(() => {
    if (state.view === "patterns" && element("pattern-type").value === "sources" && !sourceProfileId) renderPatterns();
  }, 160);
}

function setupSourceProfileDates() {
  for (const date of [...sourceDates, ...sourceComparisonDates]) {
    sourceProfileField(date).addEventListener("change", () => {
      linkNotice();
      renderPatterns();
    });
  }
}

function writeSourceProfileLink(params) {
  if (sourceProfileId) params.set("source_id", sourceProfileId);
  const fields = patternComparison.scope ? [...sourceDates, ...sourceComparisonDates] : sourceDates;
  for (const field of fields) {
    const value = sourceProfileField(field).value;
    if (value) params.set(field, value);
  }
}

function restoreSourceProfileLink(params) {
  sourceProfileId = params.get("source_id") ?? "";
  let valid = true;
  let open = false;
  for (const field of [...sourceDates, ...sourceComparisonDates]) {
    const input = sourceProfileField(field);
    const value = params.get(field) ?? "";
    input.value = value;
    valid &&= input.value === value && input.validity.valid;
    open ||= Boolean(value);
  }
  sourceProfileField("dates").open = open;
  return valid;
}

function openSourceHistory(sourceId) {
  state.view = "patterns";
  state.patternThread = "";
  state.patternScope = defaultPatternScope;
  patternComparison.scope = "";
  patternComparison.thread = "";
  restoreSourceProfileLink(new URLSearchParams({ source_id: sourceId }));
  element("pattern-type").value = "sources";
  element("pattern-search").value = "";
  linkNotice();
  render();
  element("pattern-search").focus();
}

function renderSourceProfiles(list, scope, scopes, query) {
  updatePatternThreadFilter(scopedPatternGroups(data.source_profiles, scope.organization, scope.user));
  const second = comparisonPatternFilter(scopes, data.source_profiles);
  sourceProfileField("compare-dates").hidden = !second;

  const primary = patternScopeSelection(scope, state.patternThread, sourceDates.map(sourceProfileField));
  const compare = second && patternScopeSelection(second.scope, second.thread, sourceComparisonDates.map(sourceProfileField));
  if (!primary || (second && !compare)) {
    const side = second ? " for Scope " + (primary ? "B" : "A") : "";
    list.append(node("p", "Enter complete dates" + side + ". The From date must precede the Before date.", "muted"));
    return;
  }

  const captions = second ? [patternScopeCaption(scope, element("pattern-thread")),
    patternScopeCaption(second.scope, element("pattern-compare-thread"))] : [];
  const spec = { version: 1, ...primary, search: query, limit: 20 };
  if (compare) spec.compare = compare;
  const results = node("div");
  list.append(results);
  function load(offset = 0, compareOffset = 0, focus = false) {
    const selected = sourceProfileId;
    const request = { ...spec, offset };
    if (selected) request.source_id = selected;
    if (selected && compare) request.compare_offset = compareOffset;
    showPatternPage(results, "/api/context-sources", request, (page) => {
      if (selected) {
        const card = node("article", undefined, "panel source-history");
        const back = investigationButton("All sources", () => {
          sourceProfileId = "";
          load(0, 0, true);
        });
        back.className = "occurrence";
        card.append(back, node("h2", selected));
        const display = (column, side, index) => {
          appendSourceProfileSummary(column, side.source);
          renderSourceOccurrences(column, side.rows);
          patternPager(column, index ? compareOffset : offset, side.next_offset, (next) => {
            load(index ? offset : next, index ? next : compareOffset, true);
          });
        };
        if (compare) {
          card.append(patternScopeColumns(captions, [page.scope_a, page.scope_b], display, "No recorded occurrences"));
        } else if (page.source) {
          display(card, page, 0);
        } else {
          card.append(node("p", "No recorded occurrences match this scope and filter.", "muted"));
        }
        results.append(card);
      } else {
        for (const profile of page.rows) {
          const card = node("article", undefined, "panel pattern source-history");
          card.append(node("h2", profile.source_id));
          if (compare) {
            card.append(patternScopeColumns(captions, [profile.scope_a, profile.scope_b], appendSourceProfileSummary, "No recorded occurrences"));
          } else {
            appendSourceProfileSummary(card, profile);
          }
          card.append(investigationButton("Open source occurrences", () => {
            sourceProfileId = profile.source_id;
            load(0, 0, true);
          }));
          results.append(card);
        }
        if (!page.rows.length) results.append(node("p", "No named context sources match this scope and filter.", "muted"));
        patternPager(results, offset, page.next_offset, (next) => load(next, 0, true));
      }
      const heading = results.querySelector("h2");
      if (focus && heading) {
        heading.tabIndex = -1;
        heading.focus();
      }
    });
  }
  load();
}

function appendSourceProfileSummary(container, profile) {
  const counts = [countLabel(profile.call_count, "call")];
  if (profile.conversation_count) counts.push(countLabel(profile.conversation_count, "known thread"));
  counts.push(countLabel(profile.linked_decision_count, "linked decision"));
  container.append(node("p", counts.join(" · "), "count"));
  container.append(node("p", countLabel(profile.occurrence_count, "recorded occurrence") + " · "
    + profile.replayed_occurrences + " with a measured replay · " + profile.masked_occurrences + " with a source mask", "muted"));

  const versions = node("ul", undefined, "source-version-list");
  for (const version of profile.versions) {
    const item = node("li");
    const values = [version.included_occurrences + " included", version.unavailable_occurrences + " unavailable"];
    if (version.captured_forms > 1) values.push(version.captured_forms + " captured forms");
    item.append(node("strong", version.source_version ?? "Version unavailable"), " · " + values.join(" · "));
    versions.append(item);
  }
  container.append(versions);
}

function renderSourceOccurrences(container, occurrences) {
  if (!occurrences.length) {
    container.append(node("p", "No occurrences on this page.", "muted"));
    return;
  }
  const scroll = node("div", undefined, "effects-scroll");
  const table = node("table", undefined, "effects-table");
  const heading = table.createTHead().insertRow();
  for (const label of ["Thread and call", "Version", "Availability", "Decision evidence"]) {
    const header = node("th", label);
    header.scope = "col";
    heading.append(header);
  }
  const rows = table.createTBody();
  for (const item of occurrences) {
    const row = rows.insertRow();
    const thread = [item.organization_id, item.user_id, item.conversation_id || item.run_id].filter(Boolean).join(" / ");
    const call = row.insertCell();
    call.append(patternEvidenceLink(item.inspector_path, thread));
    const provenance = [item.started_at, item.turn_id, item.trace_source, item.call_id];
    call.append(node("small", provenance.filter(Boolean).join(" · "), "anchor"));
    const version = row.insertCell();
    version.textContent = item.source_version ?? "Unavailable";
    if (item.text_sha256) {
      const digest = node("small", "Text " + item.text_sha256.slice(0, 12), "anchor");
      digest.title = "Text SHA-256 " + item.text_sha256;
      version.append(digest);
    }
    row.insertCell().textContent = labels[item.availability];
    const evidence = row.insertCell();
    evidence.textContent = countLabel(item.linked_decision_ids.length, "linked decision");
    if (item.replay_example) {
      evidence.append(sourceProfileEvidenceLink(item.replay_example.inspector_path, "Open replay · " + effectText(item.replay_example.difference_pp)));
    }
    const masks = masksBySourceItem.get(sourceItemKey(item)) || [];
    appendSourceMaskEvidence(evidence, masks);
    if (!masks.length && item.mask_example) {
      const { inspector_path, output_token, change_in_log_probability } = item.mask_example;
      evidence.append(sourceProfileEvidenceLink(inspector_path, "Largest mask effect at token " + (output_token + 1)
        + " · " + visibilityEffectFormat.format(change_in_log_probability)));
    }
  }
  scroll.append(table);
  container.append(scroll);
}

function sourceProfileEvidenceLink(path, label) {
  const row = node("div", undefined, "visibility-pattern-links");
  row.append(patternEvidenceLink(path, label));
  return row;
}
