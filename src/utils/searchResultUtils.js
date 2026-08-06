import { rotateNormalizedRectForViewport } from "./pdfGeometry.js";

const SUPPORTED_ROTATIONS = new Set([0, 90, 180, 270]);

function finiteNumber(value) {
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

function clamp(value, min, max) {
  return Math.min(Math.max(value, min), max);
}

function cleanNumber(value) {
  return Number(value.toFixed(12));
}

function createBox(x, y, width, height) {
  return {
    x: cleanNumber(x),
    y: cleanNumber(y),
    width: cleanNumber(width),
    height: cleanNumber(height),
  };
}

export function normalizeSearchRotation(rotation) {
  const normalized = ((Number(rotation) % 360) + 360) % 360;
  return SUPPORTED_ROTATIONS.has(normalized) ? normalized : 0;
}

export function sanitizeSearchBox(box) {
  if (!box || typeof box !== "object") return null;

  const x = finiteNumber(box.x);
  const y = finiteNumber(box.y);
  const width = finiteNumber(box.width ?? box.w);
  const height = finiteNumber(box.height ?? box.h);

  if (
    x === null ||
    y === null ||
    width === null ||
    height === null ||
    width <= 0 ||
    height <= 0
  ) {
    return null;
  }

  const left = clamp(x, 0, 1);
  const top = clamp(y, 0, 1);
  const right = clamp(x + width, 0, 1);
  const bottom = clamp(y + height, 0, 1);

  if (right <= left || bottom <= top) return null;

  return createBox(left, top, right - left, bottom - top);
}

export function rotateSearchBox(box, rotation = 0) {
  const sanitized = sanitizeSearchBox(box);
  if (!sanitized) return null;

  const rotated = rotateNormalizedRectForViewport(sanitized, rotation);
  if (!rotated) return null;

  return createBox(rotated.x, rotated.y, rotated.width, rotated.height);
}

export function positionSearchBox(
  box,
  {
    rotation = 0,
    pageCssWidth = 0,
    pageCssHeight = 0,
  } = {},
) {
  const rotated = rotateSearchBox(box, rotation);
  const surfaceWidth = finiteNumber(pageCssWidth);
  const surfaceHeight = finiteNumber(pageCssHeight);

  if (
    !rotated ||
    surfaceWidth === null ||
    surfaceHeight === null ||
    surfaceWidth <= 0 ||
    surfaceHeight <= 0
  ) {
    return null;
  }

  return {
    ...rotated,
    left: cleanNumber(rotated.x * surfaceWidth),
    top: cleanNumber(rotated.y * surfaceHeight),
    width: cleanNumber(rotated.width * surfaceWidth),
    height: cleanNumber(rotated.height * surfaceHeight),
    rotation: normalizeSearchRotation(rotation),
  };
}

export function normalizeSearchResponse(response) {
  const rawMatches = Array.isArray(response?.matches) ? response.matches : [];
  const results = [];
  let droppedBoxes = 0;

  for (const rawMatch of rawMatches) {
    const pageIndex = Number(rawMatch?.page_index);
    const rawBoxes = Array.isArray(rawMatch?.rects) ? rawMatch.rects : [];
    const boxes = [];

    for (const rawBox of rawBoxes) {
      const box = sanitizeSearchBox(rawBox);
      if (box) {
        boxes.push(box);
      } else {
        droppedBoxes += 1;
      }
    }

    if (!Number.isInteger(pageIndex) || pageIndex < 0 || boxes.length === 0) {
      continue;
    }

    results.push({
      index: results.length,
      rustIndex: Number.isInteger(rawMatch?.global_match_index)
        ? rawMatch.global_match_index
        : null,
      pageIndex,
      boxes,
      text: typeof rawMatch?.snippet === "string" ? rawMatch.snippet : "",
    });
  }

  const rustTotalMatches = Number.isInteger(response?.total_matches)
    ? response.total_matches
    : rawMatches.length;

  return {
    results,
    totalMatches: results.length,
    stats: {
      rustTotalMatches,
      rawMatches: rawMatches.length,
      normalizedMatches: results.length,
      droppedMatches: rawMatches.length - results.length,
      droppedBoxes,
    },
  };
}

export function groupSearchMatchesByPage(results, activeIndex = 0) {
  if (!Array.isArray(results)) return {};

  const grouped = {};
  for (const match of results) {
    if (!match || !Number.isInteger(match.pageIndex)) continue;
    if (!grouped[match.pageIndex]) grouped[match.pageIndex] = [];
    grouped[match.pageIndex].push({
      ...match,
      isActive: match.index === activeIndex,
    });
  }
  return grouped;
}

export function updateActiveSearchMatchesByPage(
  grouped,
  results,
  previousIndex,
  nextIndex,
) {
  if (!grouped || typeof grouped !== "object" || !Array.isArray(results)) {
    return {};
  }

  const previousPage = results[previousIndex]?.pageIndex;
  const nextPage = results[nextIndex]?.pageIndex;
  if (!Number.isInteger(nextPage)) return grouped;

  const pagesToUpdate = new Set([previousPage, nextPage].filter(Number.isInteger));
  const nextGrouped = { ...grouped };

  for (const pageIndex of pagesToUpdate) {
    const pageMatches = grouped[pageIndex];
    if (!Array.isArray(pageMatches)) continue;
    nextGrouped[pageIndex] = pageMatches.map((match) => ({
      ...match,
      isActive: match.index === nextIndex,
    }));
  }

  return nextGrouped;
}

export function clampSearchIndex(index, totalMatches) {
  if (!Number.isInteger(totalMatches) || totalMatches <= 0) return 0;
  const numericIndex = Number.isInteger(index) ? index : 0;
  return clamp(numericIndex, 0, totalMatches - 1);
}

export function wrapSearchIndex(index, delta, totalMatches) {
  if (!Number.isInteger(totalMatches) || totalMatches <= 0) return 0;
  const current = clampSearchIndex(index, totalMatches);
  const step = Number.isInteger(delta) ? delta : 0;
  return ((current + step) % totalMatches + totalMatches) % totalMatches;
}

export function claimHighlightScroll(
  completedKeys,
  searchRequestId,
  currentMatchIndex,
  pageIndex,
) {
  if (
    !(completedKeys instanceof Set) ||
    !Number.isInteger(searchRequestId) ||
    !Number.isInteger(currentMatchIndex) ||
    !Number.isInteger(pageIndex)
  ) {
    return false;
  }

  const key = `${searchRequestId}:${currentMatchIndex}:${pageIndex}`;
  if (completedKeys.has(key)) return false;
  completedKeys.add(key);
  return true;
}
