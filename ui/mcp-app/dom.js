export function element(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}

export function button(label, action) {
  const control = element("button", label);
  control.type = "button";
  control.onclick = action;
  return control;
}

export function countLabel(count, noun) {
  return count + " " + noun + (count === 1 ? "" : "s");
}

export function scopeColumns(parent, args, sides, show, empty) {
  const columns = element("div", undefined, "visibility-columns");
  sides.forEach((side, index) => {
    const section = element("section");
    section.append(element("h3", index ? "Scope B" : "Scope A"));
    if (args.version === 1) {
      const { scope, thread, after, before } = (index ? args.compare : args) || {};
      const identity = [scope?.organization_id ?? thread?.organization_id, scope?.user_id ?? thread?.user_id]
        .filter(Boolean).join(" / ") || "Workspace";
      const period = [after && "From " + after, before && (after ? "before " : "Before ") + before].filter(Boolean).join(" ");
      const caption = [identity, thread?.conversation_id, period].filter(Boolean).join(" · ");
      section.append(element("p", caption, "muted"));
    }
    if (side) show(section, side, index);
    else section.append(element("p", empty, "muted"));
    columns.append(section);
  });
  parent.append(columns);
}
