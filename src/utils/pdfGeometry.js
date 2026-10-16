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


function clampUnit(value) {
  return Math.min(1, Math.max(0, value));
}

/**
 * Ekran (client) noktasını, CSS ile döndürülmüş/ölçeklenmiş bir katmanın yerel,
 * normalize [0..1] koordinatına çevirir.
 *
 * `getBoundingClientRect` döndürülmüş bir elemanın yerel kutusunu değil, eksen hizalı
 * sınır kutusunu verir; tüm atalardaki ölçek (ör. `scale(cssScale)`, `scale(visualScale)`)
 * de bu kutuya yansır. Katmanın kendi `rotate(θ)` dönüşümü merkez etrafında olduğundan
 * kutunun merkezi katmanın merkezidir: nokta merkeze göre −θ döndürülür, toplam ölçeğe
 * bölünür ve transformsuz yerel boyuta (`offsetWidth/offsetHeight`) göre normalize edilir.
 *
 * Sonuç [0..1] dışına taşabilir (katman dışındaki nokta); kısma çağıranın işidir.
 *
 * @param {number} clientX
 * @param {number} clientY
 * @param {{
 *   boundingRect: { left: number, top: number, width: number, height: number },
 *   localWidth: number,
 *   localHeight: number,
 *   rotation?: number,
 * }} frame `rotation`: katmana uygulanan CSS `rotate()` açısı (derece, saat yönü).
 * @returns {{ x: number, y: number } | null}
 */
export function clientPointToLayerNormalized(clientX, clientY, frame) {
  const left = finiteNumber(frame?.boundingRect?.left);
  const top = finiteNumber(frame?.boundingRect?.top);
  const boundsWidth = finiteNumber(frame?.boundingRect?.width);
  const boundsHeight = finiteNumber(frame?.boundingRect?.height);
  const localWidth = finiteNumber(frame?.localWidth);
  const localHeight = finiteNumber(frame?.localHeight);
  const rotation = finiteNumber(frame?.rotation ?? 0);
  if (
    finiteNumber(clientX) === null ||
    finiteNumber(clientY) === null ||
    left === null ||
    top === null ||
    boundsWidth === null ||
    boundsHeight === null ||
    rotation === null ||
    !(boundsWidth > 0) ||
    !(boundsHeight > 0) ||
    !(localWidth > 0) ||
    !(localHeight > 0)
  ) {
    return null;
  }

  const radians = (rotation * Math.PI) / 180;
  const cos = Math.cos(radians);
  const sin = Math.sin(radians);
  // Döndürülmüş w×h kutusunun sınır genişliği s·(w|cos θ| + h|sin θ|)'dır.
  const scale = boundsWidth / (localWidth * Math.abs(cos) + localHeight * Math.abs(sin));
  if (!Number.isFinite(scale) || scale <= 0) return null;

  const dx = clientX - (left + boundsWidth / 2);
  const dy = clientY - (top + boundsHeight / 2);
  // CSS rotate(θ), y aşağı ekran uzayında saat yönündedir: ekran = s·R(θ)·yerel.
  const localX = (dx * cos + dy * sin) / scale;
  const localY = (-dx * sin + dy * cos) / scale;

  return {
    x: (localX + localWidth / 2) / localWidth,
    y: (localY + localHeight / 2) / localHeight,
  };
}

/**
 * İki normalize nokta arasındaki dikdörtgen; noktalar önce [0..1]'e kısılır.
 * @returns {{ x: number, y: number, width: number, height: number }}
 */
export function normalizedRectBetween(start, end) {
  const x0 = clampUnit(start.x);
  const y0 = clampUnit(start.y);
  const x1 = clampUnit(end.x);
  const y1 = clampUnit(end.y);
  return {
    x: Math.min(x0, x1),
    y: Math.min(y0, y1),
    width: Math.abs(x1 - x0),
    height: Math.abs(y1 - y0),
  };
}

/**
 * Sol-üst köşesi `point` olan, PDF point cinsinden sabit boyutlu normalize dikdörtgen.
 * Sayfa sınırına taşarsa içeri kaydırılır. `pageWidthPt/pageHeightPt` görüntü
 * uzayındaki sayfa boyutudur (page /Rotate uygulanmış).
 * @returns {{ x: number, y: number, width: number, height: number } | null}
 */
export function fixedSizeNormalizedRect(point, sizePt, pageWidthPt, pageHeightPt) {
  if (!(sizePt > 0) || !(pageWidthPt > 0) || !(pageHeightPt > 0)) return null;
  const width = Math.min(1, sizePt / pageWidthPt);
  const height = Math.min(1, sizePt / pageHeightPt);
  return {
    x: Math.min(clampUnit(point.x), 1 - width),
    y: Math.min(clampUnit(point.y), 1 - height),
    width,
    height,
  };
}
