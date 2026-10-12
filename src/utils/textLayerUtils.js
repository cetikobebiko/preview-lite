const SUPPORTED_ROTATIONS = new Set([0, 90, 180, 270]);
const MIN_SCALE_X = 0.05;
const MAX_SCALE_X = 20;

export function normalizeTextRotation(rotation) {
  const normalized = ((Number(rotation) % 360) + 360) % 360;
  return SUPPORTED_ROTATIONS.has(normalized) ? normalized : 0;
}

function clamp(value, min, max) {
  return Math.min(Math.max(value, min), max);
}

function finiteNumber(value) {
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

export function sanitizeTextLayout(layout) {
  if (!layout || typeof layout !== "object") return null;

  const x = finiteNumber(layout.x);
  const y = finiteNumber(layout.y);
  const width = finiteNumber(layout.width ?? layout.w);
  const height = finiteNumber(layout.height ?? layout.h);
  const text = typeof layout.text === "string" ? layout.text.trim() : "";

  if (
    x === null ||
    y === null ||
    width === null ||
    height === null ||
    width <= 0 ||
    height <= 0 ||
    !text
  ) {
    return null;
  }

  const left = clamp(x, 0, 1);
  const top = clamp(y, 0, 1);
  const right = clamp(x + width, 0, 1);
  const bottom = clamp(y + height, 0, 1);

  if (right <= left || bottom <= top) return null;

  return {
    x: left,
    y: top,
    width: right - left,
    height: bottom - top,
    text,
  };
}

const REFERENCE_FONT_SIZE = 100;
const DEFAULT_MAX_CACHED_WIDTHS = 10_000;

/**
 * Canvas text widths scale linearly with the font size, so each text is
 * measured once at a reference size and scaled afterwards: a zoom step costs
 * no canvas measurement for words already seen. `getContext` is called once,
 * on first use; the oldest widths are dropped beyond `maxEntries`.
 */
export function createTextWidthMeasurer(
  getContext,
  { fontFamily = "sans-serif", maxEntries = DEFAULT_MAX_CACHED_WIDTHS } = {},
) {
  const referenceWidths = new Map();
  let context;

  return function measureTextWidth(text, fontSize) {
    let width = referenceWidths.get(text);
    if (width === undefined) {
      context ??= getContext?.() ?? null;
      if (!context) return Number.NaN;
      context.font = `${REFERENCE_FONT_SIZE}px ${fontFamily}`;
      width = context.measureText(text).width;
      if (referenceWidths.size >= maxEntries) {
        referenceWidths.delete(referenceWidths.keys().next().value);
      }
      referenceWidths.set(text, width);
    }
    return width * (fontSize / REFERENCE_FONT_SIZE);
  };
}

function fallbackTextWidth(text, fontSize) {
  return Math.max(text.length * fontSize * 0.5, fontSize * 0.25);
}

function safeMeasuredWidth(text, fontSize, measureTextWidth) {
  if (typeof measureTextWidth === "function") {
    try {
      const measured = Number(measureTextWidth(text, fontSize));
      if (Number.isFinite(measured) && measured > 0) return measured;
    } catch {
      // Canvas text measurement is an enhancement; geometry still has a safe fallback.
    }
  }

  return fallbackTextWidth(text, fontSize);
}

export function positionTextBox(
  layout,
  rotation,
  pageCssWidth,
  pageCssHeight,
  measureTextWidth,
) {
  const box = sanitizeTextLayout(layout);
  const surfaceWidth = finiteNumber(pageCssWidth);
  const surfaceHeight = finiteNumber(pageCssHeight);

  if (
    !box ||
    surfaceWidth === null ||
    surfaceHeight === null ||
    surfaceWidth <= 0 ||
    surfaceHeight <= 0
  ) {
    return null;
  }

  const normalizedRotation = normalizeTextRotation(rotation);
  const isSideways = normalizedRotation === 90 || normalizedRotation === 270;
  const logicalWidth = isSideways ? surfaceHeight : surfaceWidth;
  const logicalHeight = isSideways ? surfaceWidth : surfaceHeight;
  const targetWidth = box.width * logicalWidth;
  const targetHeight = box.height * logicalHeight;
  const fontSize = targetHeight;
  const measuredWidth = safeMeasuredWidth(box.text, fontSize, measureTextWidth);
  const scaleX = clamp(targetWidth / measuredWidth, MIN_SCALE_X, MAX_SCALE_X);

  let left;
  let top;
  let visualWidth;
  let visualHeight;
  let transform;

  switch (normalizedRotation) {
    case 90:
      left = (1 - box.y - box.height) * surfaceWidth;
      top = box.x * surfaceHeight;
      visualWidth = targetHeight;
      visualHeight = targetWidth;
      transform = `translateX(${targetHeight}px) rotate(90deg) scaleX(${scaleX})`;
      break;
    case 180:
      left = (1 - box.x - box.width) * surfaceWidth;
      top = (1 - box.y - box.height) * surfaceHeight;
      visualWidth = targetWidth;
      visualHeight = targetHeight;
      transform = `translate(${targetWidth}px, ${targetHeight}px) rotate(180deg) scaleX(${scaleX})`;
      break;
    case 270:
      left = box.y * surfaceWidth;
      top = (1 - box.x - box.width) * surfaceHeight;
      visualWidth = targetHeight;
      visualHeight = targetWidth;
      transform = `translateY(${targetWidth}px) rotate(270deg) scaleX(${scaleX})`;
      break;
    default:
      left = box.x * surfaceWidth;
      top = box.y * surfaceHeight;
      visualWidth = targetWidth;
      visualHeight = targetHeight;
      transform = `scaleX(${scaleX})`;
      break;
  }

  return {
    ...box,
    left,
    top,
    width: measuredWidth,
    height: targetHeight,
    visualWidth,
    visualHeight,
    fontSize,
    scaleX,
    rotation: normalizedRotation,
    transform,
  };
}

export function createTextLayerItems(
  layouts,
  {
    rotation = 0,
    pageCssWidth = 0,
    pageCssHeight = 0,
    measureTextWidth,
  } = {},
) {
  if (!Array.isArray(layouts)) return [];

  const positioned = layouts
    .map((layout) =>
      positionTextBox(
        layout,
        rotation,
        pageCssWidth,
        pageCssHeight,
        measureTextWidth,
      ),
    )
    .filter(Boolean);

  return positioned.map((item, index) => ({
    ...item,
    hasTrailingSpace: index < positioned.length - 1,
  }));
}
