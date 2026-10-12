/**
 * Pure page-index and viewport geometry rules for viewer navigation.
 *
 * Contract:
 * - displayIndex: position inside viewer.pageOrder. viewer.currentPageIndex, the page
 *   number input (displayIndex + 1), history stacks' UI and thumbnails' order use it.
 * - physicalIndex: the PDF page index. DOM ids (`page-${physicalIndex}`), PdfPage
 *   pageIndex, outline page_index, search and scan results use it.
 */

export const VISIBLE_PAGE_HYSTERESIS = 0.05;

export function toPhysicalPageIndex(pageOrder, displayIndex) {
  if (!Array.isArray(pageOrder) || !Number.isInteger(displayIndex) || displayIndex < 0 || displayIndex >= pageOrder.length) return -1;
  const physicalIndex = pageOrder[displayIndex];
  return Number.isInteger(physicalIndex) && physicalIndex >= 0 ? physicalIndex : -1;
}

export function toDisplayPageIndex(pageOrder, physicalIndex) {
  if (!Array.isArray(pageOrder) || !Number.isInteger(physicalIndex) || physicalIndex < 0) return -1;
  return pageOrder.indexOf(physicalIndex);
}

/** Parses a 1-based page number typed by the user; returns a displayIndex or -1. */
export function parsePageInput(rawValue, pageCount) {
  const text = String(rawValue ?? "").trim();
  if (!/^\d+$/.test(text)) return -1;
  const page = Number(text);
  return page >= 1 && page <= pageCount ? page - 1 : -1;
}

/**
 * Chooses the physical page that should be "current" from IntersectionObserver ratios.
 * The current page is kept until another page is more visible by `hysteresis`, so two
 * pages sharing the viewport almost equally do not flip back and forth.
 */
export function selectMostVisiblePage(visibleRatios, currentPhysicalIndex, pageOrder, hysteresis = VISIBLE_PAGE_HYSTERESIS) {
  let bestPhysical = -1;
  let bestRatio = 0;
  let bestDisplay = Infinity;
  for (const [physicalIndex, ratio] of visibleRatios) {
    if (!(ratio > 0)) continue;
    const displayIndex = toDisplayPageIndex(pageOrder, physicalIndex);
    if (displayIndex < 0) continue;
    if (ratio > bestRatio || (ratio === bestRatio && displayIndex < bestDisplay)) {
      bestPhysical = physicalIndex;
      bestRatio = ratio;
      bestDisplay = displayIndex;
    }
  }
  if (bestPhysical < 0 || bestPhysical === currentPhysicalIndex) return currentPhysicalIndex;
  const currentRatio = toDisplayPageIndex(pageOrder, currentPhysicalIndex) >= 0
    ? visibleRatios.get(currentPhysicalIndex) ?? 0
    : 0;
  if (currentRatio > 0 && bestRatio - currentRatio < hysteresis) return currentPhysicalIndex;
  return bestPhysical;
}

/**
 * Finds the document point at the viewport's vertical centre as { physicalIndex, offsetRatio }.
 * `pages` holds { physicalIndex, top, height } in scroll-content coordinates.
 */
export function findViewportAnchor(pages, scrollTop, viewportHeight) {
  const centerY = scrollTop + viewportHeight / 2;
  let nearest = null;
  let nearestDistance = Infinity;
  for (const page of pages) {
    if (!(page.height > 0)) continue;
    const bottom = page.top + page.height;
    if (centerY >= page.top && centerY <= bottom) {
      return { physicalIndex: page.physicalIndex, offsetRatio: (centerY - page.top) / page.height };
    }
    const distance = centerY < page.top ? page.top - centerY : centerY - bottom;
    if (distance < nearestDistance) {
      nearest = page;
      nearestDistance = distance;
    }
  }
  if (!nearest) return null;
  return { physicalIndex: nearest.physicalIndex, offsetRatio: centerY < nearest.top ? 0 : 1 };
}

/** Returns the scrollTop that puts the anchor's document point back at the viewport centre. */
export function resolveAnchorScrollTop(anchor, page, viewportHeight, maxScrollTop = Infinity) {
  const target = page.top + anchor.offsetRatio * page.height - viewportHeight / 2;
  return Math.min(Math.max(target, 0), Math.max(maxScrollTop, 0));
}
