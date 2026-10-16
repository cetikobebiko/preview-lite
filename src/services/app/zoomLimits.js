/**
 * Committed viewer scale bounds. Render memory guards (MIN_GESTURE_VISUAL_SCALE in viewerController.js,
 * PdfPage's `isZooming` render gate) assume a committed scale of at most MAX_SCALE.
 */
export const MIN_SCALE = 0.25;
export const MAX_SCALE = 4;

/**
 * Returns a committable scale: non-finite or non-number input becomes 1, finite input is clamped to
 * [MIN_SCALE, MAX_SCALE] and rounded to two decimals so repeated steps do not accumulate float error
 * (the value is part of PdfPage's render key and is persisted).
 */
export function normalizeScale(value) {
  if (typeof value !== "number" || !Number.isFinite(value)) return 1;
  return Math.round(Math.min(Math.max(value, MIN_SCALE), MAX_SCALE) * 100) / 100;
}
