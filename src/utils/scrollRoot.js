const SCROLLING = new Set(["auto", "scroll", "overlay"]);

/**
 * Nearest ancestor that scrolls vertically, or null for the viewport.
 *
 * An IntersectionObserver with the implicit (viewport) root clips its target
 * by every scroll container in between, so its rootMargin cannot reach rows
 * scrolled out of a sidebar or grid. Observing with the scroll container as
 * root makes the margin apply to that container's scrollport.
 */
export function findScrollRoot(element) {
  if (typeof getComputedStyle !== "function") return null;
  for (let node = element?.parentElement; node; node = node.parentElement) {
    if (SCROLLING.has(getComputedStyle(node).overflowY)) return node;
  }
  return null;
}
