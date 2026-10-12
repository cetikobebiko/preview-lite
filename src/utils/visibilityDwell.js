// Starts work only after an element has stayed visible for `delayMs`.
// Fast scrolling through a thumbnail list would otherwise begin (and then
// cancel) a render for every item that crosses the viewport.
export const THUMBNAIL_VISIBILITY_DWELL_MS = 120;

export function createVisibilityDwell({
  delayMs = THUMBNAIL_VISIBILITY_DWELL_MS,
  onDwell,
} = {}) {
  if (typeof onDwell !== "function") {
    throw new TypeError("onDwell işlevi gerekli.");
  }

  let timer = null;

  function leave() {
    if (timer === null) return;
    clearTimeout(timer);
    timer = null;
  }

  function enter() {
    if (timer !== null) return;
    timer = setTimeout(() => {
      timer = null;
      onDwell();
    }, Math.max(0, delayMs));
  }

  return Object.freeze({
    enter,
    leave,
    destroy: leave,
    isPending: () => timer !== null,
  });
}
