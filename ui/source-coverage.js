// Summarize one allowed combination of named sources on either preflight view.
function sourceAlternativeText(coverage, group) {
  const differs = group.unconfirmed.filter(id => coverage.text_mismatches?.includes(id));
  const missing = group.unconfirmed.filter(id => !differs.includes(id));
  const parts = [group.required.join(" + ")];
  if (differs.length) parts.push("Captured text differs: " + differs.join(", "));
  if (missing.length) parts.push("Still unconfirmed: " + missing.join(", "));
  if (!group.unconfirmed.length) {
    parts.push("All anchored in the request");
    const pinned = group.required.filter(id => Object.hasOwn(coverage.expected_text_sha256 || {}, id));
    if (pinned.length) parts.push("Expected text matched: " + pinned.join(", "));
  }
  return parts.join(" · ");
}
