import {
  clampNormalizedRect,
  composeRenderedPageRotation,
  rotateNormalizedRectForViewport,
} from "./pdfGeometry.js";

export const COMPARE_KIND_LABELS = Object.freeze({
  inserted: "Eklendi",
  deleted: "Silindi",
  replaced: "Değişti",
});

/** The side of a change that belongs to the open document. */
export function currentSide(change, currentIsOld) {
  return currentIsOld ? change.old : change.new;
}

/** The side of a change that belongs to the chosen file. */
export function otherSide(change, currentIsOld) {
  return currentIsOld ? change.new : change.old;
}

/**
 * Highlights on the open document's pages, keyed by physical page index. A change whose
 * words are only in the other file is marked next to its neighbouring word (`marker`).
 */
export function buildCompareHighlightsByPage(result) {
  const byPage = {};
  if (!result || !Array.isArray(result.changes)) return byPage;
  result.changes.forEach((change, changeIndex) => {
    const side = currentSide(change, result.currentIsOld);
    if (!side || typeof side.pageIndex !== "number") return;
    const rects = side.rects?.length > 0 ? side.rects : side.near ? [side.near] : [];
    const marker = !(side.rects?.length > 0);
    for (const rect of rects) {
      (byPage[side.pageIndex] ??= []).push({
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        intrinsicRotation: side.intrinsicRotation ?? 0,
        kind: change.kind,
        marker,
        changeIndex,
      });
    }
  });
  return byPage;
}

/** One page's highlights in viewport space for the viewer's rotation. */
export function selectCompareHighlights(highlightsByPage, pageIndex, viewerRotation = 0) {
  const logicals = highlightsByPage?.[pageIndex];
  if (!Array.isArray(logicals) || logicals.length === 0) return [];
  const out = [];
  for (const rect of logicals) {
    const rotation = composeRenderedPageRotation(rect.intrinsicRotation, viewerRotation);
    const rotated = rotateNormalizedRectForViewport(rect, rotation);
    const clamped = rotated && clampNormalizedRect(rotated);
    if (!clamped) continue;
    out.push({ ...clamped, kind: rect.kind, marker: rect.marker, changeIndex: rect.changeIndex });
  }
  return out;
}
