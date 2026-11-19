"use strict";

function openSourceHistory(sourceId) {
  state.view = "patterns";
  state.patternThread = "";
  state.patternScope = defaultPatternScope;
  element("pattern-type").value = "sources";
  element("pattern-search").value = sourceId;
  render();
  element("pattern-search").focus();
}

function renderSourceProfiles(list, organization, user, query) {
  const groups = scopedPatternGroups(data.source_profiles, organization, user);
  updatePatternThreadFilter(groups);
  const profiles = [];
  for (const { source_id, occurrences } of groups) {
    const scoped = occurrences.filter((item) =>
      !state.patternThread || patternThreadKey(item) === state.patternThread
    );
    if (!scoped.length) continue;
    if (!source_id.toLowerCase().includes(query) && !scoped.some((item) =>
      item.source_version?.toLowerCase().includes(query) || item.trace_source.toLowerCase().includes(query)
    )) continue;
    const calls = new Set(scoped.map((item) => JSON.stringify([item.run_id, item.call_id])));
    profiles.push({ source_id, occurrences: scoped, calls: calls.size });
  }
  profiles.sort((left, right) => right.calls - left.calls || left.source_id.localeCompare(right.source_id));
  for (const profile of profiles) list.append(sourceProfileCard(profile));
  if (!profiles.length) list.append(node("p", "No named context sources match this scope and filter.", "muted"));
}

function sourceProfileCard({ source_id, occurrences, calls }) {
  const card = node("article", undefined, "panel pattern source-history");
  const knownThreads = new Set(occurrences.map((item) => patternThreadKey(item)).filter(Boolean));
  const decisions = new Set();
  const versions = new Map();
  for (const item of occurrences) {
    const { source_version, run_id, call_id, linked_decision_ids } = item;
    for (const id of linked_decision_ids) decisions.add(JSON.stringify([run_id, call_id, id]));
    if (!versions.has(source_version)) versions.set(source_version, { included: 0, unavailable: 0, forms: new Set() });
    const version = versions.get(source_version);
    if (item.availability === "included") version.included += 1;
    else version.unavailable += 1;
    if (item.text_sha256) version.forms.add(item.text_sha256);
  }
  const counts = [countLabel(calls, "call")];
  if (knownThreads.size) counts.push(countLabel(knownThreads.size, "known thread"));
  counts.push(countLabel(decisions.size, "linked decision"));
  const measured = occurrences.filter((item) => measuredSourceItems.has(sourceItemKey(item))).length;
  counts.push(measured + " of " + occurrences.length + " context items measured");
  card.append(node("h2", source_id), node("p", counts.join(" · "), "count"));

  const versionList = node("ul", undefined, "source-version-list");
  for (const [version, summary] of versions) {
    const item = node("li");
    const label = [summary.included + " included", summary.unavailable + " unavailable"];
    if (summary.forms.size > 1) label.push(summary.forms.size + " captured forms");
    item.append(node("strong", version ?? "Version unavailable"), " · " + label.join(" · "));
    versionList.append(item);
  }
  card.append(versionList);

  const history = node("details");
  history.append(node("summary", "Open source occurrences"));
  history.addEventListener("toggle", () => {
    if (history.open && history.childElementCount === 1) {
      renderSourceOccurrences(history, [...occurrences].sort(compareTimes));
    }
  });
  card.append(history);
  return card;
}

function renderSourceOccurrences(history, occurrences) {
  const scroll = node("div", undefined, "effects-scroll");
  const table = node("table", undefined, "effects-table");
  const heading = table.createTHead().insertRow();
  for (const label of ["Thread and call", "Version", "Availability", "Decision evidence"]) {
    const header = node("th", label);
    header.scope = "col";
    heading.append(header);
  }
  const rows = table.createTBody();
  const more = node("button", "Show more occurrences");
  let offset = 0;
  const appendPage = () => {
    for (const item of occurrences.slice(offset, offset + 30)) {
      const row = rows.insertRow();
      const thread = [item.organization_id, item.user_id, item.conversation_id || item.run_id].filter(Boolean).join(" / ");
      const call = row.insertCell();
      call.append(occurrenceButton(
        { ...item, decision_id: item.linked_decision_ids[0] }, thread, { context_item_id: item.context_id }
      ));
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
      const measured = measuredSourceItems.get(sourceItemKey(item));
      if (measured) {
        evidence.append(occurrenceButton(measured.location, "Open replay · " + effectText(measured.difference_pp), measured));
      }
    }
    offset = rows.rows.length;
    more.disabled = offset >= occurrences.length;
    more.textContent = more.disabled ? offset + " occurrences shown" :
      "Show more · " + offset + " of " + occurrences.length + " shown";
    more.hidden = occurrences.length <= 30;
  };
  more.addEventListener("click", appendPage);
  appendPage();
  scroll.append(table);
  history.append(scroll, more);
}
