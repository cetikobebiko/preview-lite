const SUPPORTED_ROTATIONS = new Set([0, 90, 180, 270]);

const ALLOWED_GEOMETRY_KEYS = new Set([
  "coordinateSpace",
  "unit",
  "origin",
  "mediaBox",
  "cropBox",
  "effectiveBox",
  "intrinsicRotation",
]);

const ALLOWED_BOX_KEYS = new Set(["left", "bottom", "width", "height"]);

function finiteNumber(value) {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : null;
}

function cleanNumber(value) {
  return Number(value.toFixed(12));
}

function validatePdfBox(box) {
  if (!box || typeof box !== "object" || Array.isArray(box)) {
    return false;
  }
  for (const key of Object.keys(box)) {
    if (!ALLOWED_BOX_KEYS.has(key)) return false;
  }
  const left = finiteNumber(box.left);
  const bottom = finiteNumber(box.bottom);
  const width = finiteNumber(box.width);
  const height = finiteNumber(box.height);

  if (
    left === null ||
    bottom === null ||
    width === null ||
    height === null ||
    width <= 0 ||
    height <= 0
  ) {
    return false;
  }
  return true;
}

export function validatePageGeometry(geometry) {
  if (!geometry || typeof geometry !== "object" || Array.isArray(geometry)) {
    return false;
  }
  for (const key of Object.keys(geometry)) {
    if (!ALLOWED_GEOMETRY_KEYS.has(key)) return false;
  }

  if (geometry.coordinateSpace !== "pdf_user_space") return false;
  if (geometry.unit !== "pdf_point") return false;
  if (geometry.origin !== "bottom_left") return false;

  if (!validatePdfBox(geometry.mediaBox)) return false;
  if (
    geometry.cropBox !== null &&
    geometry.cropBox !== undefined &&
    !validatePdfBox(geometry.cropBox)
  ) {
    return false;
  }
  if (!validatePdfBox(geometry.effectiveBox)) return false;

  if (
    typeof geometry.intrinsicRotation !== "number" ||
    !SUPPORTED_ROTATIONS.has(geometry.intrinsicRotation)
  ) {
    return false;
  }

  return true;
}

export function pdfPointRectToNormalizedTopLeft(pdfRect, pageGeometry) {
  if (!validatePageGeometry(pageGeometry)) return null;
  if (!pdfRect || typeof pdfRect !== "object" || Array.isArray(pdfRect)) {
    return null;
  }

  const x = finiteNumber(pdfRect.x);
  const y = finiteNumber(pdfRect.y);
  const width = finiteNumber(pdfRect.width);
  const height = finiteNumber(pdfRect.height);

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

  const box = pageGeometry.effectiveBox;
  const pageTop = box.bottom + box.height;

  const normalizedX = (x - box.left) / box.width;
  const normalizedY = (pageTop - (y + height)) / box.height;
  const normalizedWidth = width / box.width;
  const normalizedHeight = height / box.height;

  if (
    !Number.isFinite(normalizedX) ||
    !Number.isFinite(normalizedY) ||
    !Number.isFinite(normalizedWidth) ||
    !Number.isFinite(normalizedHeight)
  ) {
    return null;
  }

  return {
    space: "normalized_page",
    x: cleanNumber(normalizedX),
    y: cleanNumber(normalizedY),
    width: cleanNumber(normalizedWidth),
    height: cleanNumber(normalizedHeight),
  };
}

export function normalizedTopLeftRectToPdfPointRect(normalizedRect, pageGeometry) {
  if (!validatePageGeometry(pageGeometry)) return null;
  if (
    !normalizedRect ||
    typeof normalizedRect !== "object" ||
    Array.isArray(normalizedRect)
  ) {
    return null;
  }

  const nx = finiteNumber(normalizedRect.x);
  const ny = finiteNumber(normalizedRect.y);
  const nw = finiteNumber(normalizedRect.width);
  const nh = finiteNumber(normalizedRect.height);

  if (nx === null || ny === null || nw === null || nh === null || nw <= 0 || nh <= 0) {
    return null;
  }

  const box = pageGeometry.effectiveBox;
  const pageTop = box.bottom + box.height;

  const pdfX = box.left + nx * box.width;
  const pdfY = pageTop - (ny + nh) * box.height;
  const pdfWidth = nw * box.width;
  const pdfHeight = nh * box.height;

  if (
    !Number.isFinite(pdfX) ||
    !Number.isFinite(pdfY) ||
    !Number.isFinite(pdfWidth) ||
    !Number.isFinite(pdfHeight)
  ) {
    return null;
  }

  return {
    x: cleanNumber(pdfX),
    y: cleanNumber(pdfY),
    width: cleanNumber(pdfWidth),
    height: cleanNumber(pdfHeight),
  };
}

export function clampNormalizedRect(rect) {
  if (!rect || typeof rect !== "object" || Array.isArray(rect)) {
    return null;
  }

  const x = finiteNumber(rect.x);
  const y = finiteNumber(rect.y);
  const width = finiteNumber(rect.width ?? rect.w);
  const height = finiteNumber(rect.height ?? rect.h);

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

  const left = Math.max(0, x);
  const top = Math.max(0, y);
  const right = Math.min(1, x + width);
  const bottom = Math.min(1, y + height);

  const clampedWidth = right - left;
  const clampedHeight = bottom - top;

  if (clampedWidth <= 0 || clampedHeight <= 0 || left >= right || top >= bottom) {
    return null;
  }

  return {
    space: "normalized_page",
    x: cleanNumber(left),
    y: cleanNumber(top),
    width: cleanNumber(clampedWidth),
    height: cleanNumber(clampedHeight),
  };
}

export function rotateNormalizedRectForViewport(rect, viewerRotation = 0) {
  if (!rect || typeof rect !== "object" || Array.isArray(rect)) {
    return null;
  }

  const x = finiteNumber(rect.x);
  const y = finiteNumber(rect.y);
  const width = finiteNumber(rect.width ?? rect.w);
  const height = finiteNumber(rect.height ?? rect.h);

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

  const rot = ((Number(viewerRotation) % 360) + 360) % 360;
  const normalizedRotation = SUPPORTED_ROTATIONS.has(rot) ? rot : 0;

  switch (normalizedRotation) {
    case 90:
      return {
        space: "normalized_page",
        x: cleanNumber(1 - y - height),
        y: cleanNumber(x),
        width: cleanNumber(height),
        height: cleanNumber(width),
      };
    case 180:
      return {
        space: "normalized_page",
        x: cleanNumber(1 - x - width),
        y: cleanNumber(1 - y - height),
        width: cleanNumber(width),
        height: cleanNumber(height),
      };
    case 270:
      return {
        space: "normalized_page",
        x: cleanNumber(y),
        y: cleanNumber(1 - x - width),
        width: cleanNumber(height),
        height: cleanNumber(width),
      };
    default:
      return {
        space: "normalized_page",
        x: cleanNumber(x),
        y: cleanNumber(y),
        width: cleanNumber(width),
        height: cleanNumber(height),
      };
  }
}

export function composeRenderedPageRotation(intrinsicRotation = 0, viewerRotation = 0) {
  const iRot = ((Number(intrinsicRotation) % 360) + 360) % 360;
  const vRot = ((Number(viewerRotation) % 360) + 360) % 360;
  const combined = (iRot + vRot) % 360;
  return SUPPORTED_ROTATIONS.has(combined) ? combined : 0;
}

export function buildLogicalHighlightsByPage(report) {
  const result = {};
  if (!report || !Array.isArray(report.items)) return result;

  for (const item of report.items) {
    if (item.geometryStatus !== "resolved" || !item.pageGeometry) continue;
    const pageIndex = item.pageIndex;
    if (typeof pageIndex !== "number") continue;

    if (!Array.isArray(item.sourceRectsPdfPoints)) continue;

    for (const pdfRect of item.sourceRectsPdfPoints) {
      const logical = pdfPointRectToNormalizedTopLeft(pdfRect, item.pageGeometry);
      if (!logical) continue;
      if (!result[pageIndex]) {
        result[pageIndex] = [];
      }
      result[pageIndex].push({
        ...logical,
        intrinsicRotation: item.pageGeometry.intrinsicRotation ?? 0,
      });
    }
  }
  return result;
}

export function selectViewportHighlights(logicalHighlightsByPage, pageIndex, viewerRotation = 0) {
  if (!logicalHighlightsByPage || typeof pageIndex !== "number") return [];
  const logicals = logicalHighlightsByPage[pageIndex];
  if (!Array.isArray(logicals) || logicals.length === 0) return [];

  const viewportHighlights = [];
  for (const rect of logicals) {
    const effectiveRot = composeRenderedPageRotation(rect.intrinsicRotation, viewerRotation);
    const rotated = rotateNormalizedRectForViewport(rect, effectiveRot);
    if (!rotated) continue;
    const clamped = clampNormalizedRect(rotated);
    if (!clamped) continue;
    viewportHighlights.push(clamped);
  }

  return viewportHighlights;
}

