// Builds range text for the split dialog ("1-3, 4-6, 7"), in display positions (1-based).

const rangeText = (start, end) => (start === end ? String(start) : `${start}-${end}`);

function rangesFromStarts(starts, pageCount) {
  return starts
    .map((start, index) => rangeText(start, (starts[index + 1] ?? pageCount + 1) - 1))
    .join(", ");
}

/** One range per `size` pages; the last range takes what is left. */
export function everyNPagesRangesText(pageCount, size) {
  const step = Math.floor(Number(size));
  if (!Number.isInteger(pageCount) || pageCount < 1 || !Number.isInteger(step) || step < 1) return "";
  const starts = [];
  for (let start = 1; start <= pageCount; start += step) starts.push(start);
  return rangesFromStarts(starts, pageCount);
}

/**
 * One range per top-level bookmark, from its page up to the next one's. Pages before the
 * first bookmark form a range of their own. `outline` nodes carry physical `page_index`;
 * `pageOrder` maps display positions to physical pages (null: identity).
 */
export function bookmarkRangesText(outline, pageCount, pageOrder = null) {
  if (!Number.isInteger(pageCount) || pageCount < 1) return "";
  const toDisplay = (physical) => (pageOrder ? pageOrder.indexOf(physical) : physical);
  const starts = [...new Set((outline ?? [])
    .map((node) => toDisplay(node?.page_index))
    .filter((position) => Number.isInteger(position) && position >= 0 && position < pageCount)
    .map((position) => position + 1))]
    .sort((a, b) => a - b);
  if (starts.length === 0) return "";
  if (starts[0] !== 1) starts.unshift(1);
  return rangesFromStarts(starts, pageCount);
}
