// Preserve button identity across unchanged background status updates.
const renderedMarkup = new WeakMap<Element, string>();
export function setMarkupIfChanged(element: Element, markup: string): void {
  if (renderedMarkup.get(element) === markup) return;
  element.innerHTML = markup;
  renderedMarkup.set(element, markup);
}
