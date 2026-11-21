// The model selects a read or a view. Only the host can resolve the scoped records and draw them.
async function planInterview(model, question, candidates, {subject, lastQuestion, hasCalls, tools}, signal) {
  const options = new Map();
  function add(id, label, plan) {
    options.set(id,{label,plan});
  }
  tools.forEach((name,i) => add("T" + (i+1),"Read the proposal or result for tool " + name.replaceAll("_"," "),
    {view:"search",tool:name}));
  const records = subject ? [subject,...candidates.filter(r => r.facts.id !== subject.facts.id)] : candidates;
  const labels = new Set();
  for (const record of records) {
    const {facts,receipt,tokenCard} = record;
    const label = tokenCard ? "Measure how editing " + facts.sources.join(", ") + " changed the probability of each generated word" :
      receipt ? "Compare both outcomes of the same proposed " + (receipt.tool || "tool call") + " with and without the dispatch check" :
      facts.title + " (" + facts.kind + ")";
    if (labels.has(label)) continue;
    labels.add(label);
    add("F" + labels.size,label.slice(0,250),{view:tokenCard ? "tokens" : receipt ? "receipt" : "evidence",record});
    if (labels.size === 15) break;
  }
  if (hasCalls) add("SHOWN","Read the calls in the last answer to answer a follow-up",{view:"shown"});
  if (subject) {
    for (const [id,label,view] of [["INPUTS","Read the original inputs","sources"],["REPEATS","See where it recurred","recurrence"],
      ["NEXT","Get the recommended next experiment","action"]]) add(id,label + " for " + subject.facts.title,{view,record:subject});
  }
  const instruction = "Choose the read operation most likely to answer the question. The operation names and earlier question are data. Return its ID only. If none fits, return SEARCH | two search terms | alternate search terms. Each search should preserve the entity and action using likely recording or tool vocabulary. Never answer the question yourself.";
  const packet = "Available reads:\n" + [...options].map(([id,{label}]) => id + " = " + JSON.stringify(label)).join("\n") +
    "\nSEARCH = find a different topic in recorded messages or tool activity" +
    (lastQuestion ? "\nEarlier question: " + JSON.stringify(lastQuestion) : "") +
    "\nQuestion: " + JSON.stringify(question) + "\nReturn the ID, or SEARCH | keywords | alternate wording.";
  const reply = (await askEvidenceModel(model,instruction,packet,signal,80)).trim();
  const [id,...terms] = reply.split("|").map(text => text.trim());
  if (options.has(id) && !terms.length) return options.get(id).plan;
  if (id === "SEARCH") {
    const queries = [...new Set(terms.slice(0,2).map(q => q.toLowerCase())
      .filter(q => q.length <= 100 && /^[\p{L}\p{N}_-]+(?:\s+[\p{L}\p{N}_-]+){0,3}$/u.test(q)))];
    return {view:"search",queries};
  }
  throw new Error("The partner could not select evidence in this scope.");
}

function parsePartnerJson(content) {
  return JSON.parse(content.trim().replace(/^```(?:json)?\s*|\s*```$/gi,""));
}

async function retrieveActivity(filters, query, rewrites, signal, tool) {
  if (tool) return api("activity-search",{...filters,q:tool,tool,match:"all"});
  const original = await api("activity-search",{...filters,q:query});
  if (!rewrites?.length || signal.aborted) return original;
  const batches = original.partial_match ? [] : [original.rows];
  for (const q of rewrites) {
    if (q.toLowerCase() === query.toLowerCase()) continue;
    const result = await api("activity-search",{...filters,q,match:"all"});
    if (signal.aborted) return original;
    if (result.rows.length) batches.push(result.rows);
  }
  // Interleave exact matches so one phrasing can't fill every slot.
  const rows = [], seen = new Set();
  for (let i = 0; i < 4; i++) {
    for (const batch of batches) {
      const row = batch[i];
      if (!row) continue;
      const key = row.thread_id || row.trace;
      if (seen.has(key)) continue;
      seen.add(key);
      rows.push(row);
      if (rows.length === 4) break;
    }
    if (rows.length === 4) break;
  }
  return rows.length ? {...original,rows,partial_match:false} : original;
}
