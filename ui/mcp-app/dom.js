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
