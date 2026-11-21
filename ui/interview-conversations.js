// Each displayed choice retains its scope and the call that supported the finding.
async function readConversationChoices(request,offset) {
  const {filters,match} = request;
  const occurrences = Boolean(match.kind || match.group);
  const page = await api(occurrences ? "occurrences" : "threads",{
    ...filters,...match,...(occurrences && {by_thread:1,fields:"reference"}),offset,limit:3,
  });
  if (match.name && !offset) {
    const exact = page.rows.filter(row => row.thread.toLowerCase() === match.name.toLowerCase());
    if (exact.length === 1 || (page.rows.length === 1 && page.next_offset == null)) return {unique:exact[0] || page.rows[0]};
  }
  const sections = page.rows.map(row => {
    const title = row.thread || "Recorded conversation";
    const conversation = {thread:{id:row.thread_key || row.id,thread:title},filters,
      focus:row.trace && row.call ? {trace:row.trace,call:row.call} : null};
    const lines = occurrences ? [] : [row.org,row.user,plural(row.trace_count,"recorded request"),fullDate(row.start)].filter(Boolean);
    return {title,lines,conversation,action:"Read " + title};
  });
  if (match.name && !sections.length) return {title:"No matching conversation",description:"Try using part of the recorded conversation name.",sections};
  const description = request.description || (sections.length ? "Each conversation opens at a recorded occurrence." :
    "No matching conversation with a recorded ID was found in this scope.");
  return {title:request.title,description,sections,more:page.next_offset == null ? null : {request,offset:page.next_offset}};
}

// Ordinals refer to the choices in the last answer, in their displayed order.
function listedConversationIndex(question, length) {
  const text = question.trim().toLowerCase().replace(/[?.!]+$/, "");
  const match = /^(?:(?:please|can you|could you) )?(?:(?:open|read|show|inspect|select|take me to)(?: me)?\s+)?(?:the\s+)?(?:conversation\s+#?)?(first|second|third|fourth|fifth|last|\d+(?:st|nd|rd|th)?)(?:\s+(?:one|conversation|example|result))?$/.exec(text);
  if (!match) return null;
  if (match[1] === "last") return length - 1;
  const words = ["first","second","third","fourth","fifth"];
  const ordinal = words.indexOf(match[1]);
  return ordinal >= 0 ? ordinal : parseInt(match[1],10) - 1;
}

function listedConversationName(question,sections) {
  const match = /^(?:open|read|show|inspect)\s+(?:(?:the )?conversation\s+)?(.+?)\s*[?!.]*$/i.exec(question.trim());
  if (!match) return null;
  const title = match[1].replace(/^["“']|[”"']$/g,"").toLowerCase();
  const found = sections.filter(section => section.title.toLowerCase() === title);
  return found.length === 1 ? sections.indexOf(found[0]) : found.length ? -1 : null;
}

function conversationTheme(question) {
  const match = /^(?:(?:which|what) (?:conversations|sessions) (?:had|have|contained|showed)|(?:show(?: me)?|list) (?:the )?(?:conversations|sessions) with)\s+(.+?)[?!.]*$/i.exec(question.trim());
  if (!match) return null;
  const label = match[1].toLowerCase().replace(/^(?:a|an) /,"");
  return Object.keys(kindLabels).find(key => {
    const name = kindLabels[key].toLowerCase();
    return label === name || label === name + "s";
  }) || (label === "repeated inputs" ? "duplicate" : null);
}

function chartConversationRequest(question, chart) {
  if (!chart) return null;
  const text = question.toLowerCase().trim().replace(/[?!.]+$/, "");
  if (chart.choosing) {
    const index = listedConversationIndex(text,chart.themes.length);
    if (index !== null) return {themes:chart.themes[index] ? [chart.themes[index]] : chart.themes};
    const named = chart.themes.find(row => kindLabels[row.kind]?.toLowerCase() === text);
    if (named) return {themes:[named]};
  }
  const counted = /^(?:which|show(?: me)?) (?:the |those )?(\d+|one|two|three|four|five|six|seven|eight|nine|ten) (?:conversations|sessions|ones)$/.exec(text);
  if (counted) {
    const words = ["zero","one","two","three","four","five","six","seven","eight","nine","ten"];
    const count = Number.isNaN(Number(counted[1])) ? words.indexOf(counted[1]) : Number(counted[1]);
    return {count,themes:chart.themes.filter(row => row.conversations === count)};
  }
  return /^(?:which (?:ones|conversations|sessions)(?: (?:were (?:those|affected)|had (?:that|it)))?|show (?:me )?those (?:conversations|sessions))$/.test(text) ?
    {themes:chart.themes} : null;
}
