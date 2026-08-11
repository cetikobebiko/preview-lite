import { rotateNormalizedRectForViewport } from "../../utils/pdfGeometry.js";

const SUPPORTED_ROTATIONS = new Set([0, 90, 180, 270]);

function normalizedRotation(value) {
  const rotation = ((Number(value) % 360) + 360) % 360;
  return SUPPORTED_ROTATIONS.has(rotation) ? rotation : 0;
}

function clampUnit(value) {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.min(1, Math.max(0, value));
}

const cleanNumber = (value) => Number(value.toFixed(12));

export function viewportPointToLogicalNormalized(point, viewerRotation = 0) {
  const viewportX = clampUnit(point?.x);
  const viewportY = clampUnit(point?.y);
  if (viewportX === null || viewportY === null) return null;

  switch (normalizedRotation(viewerRotation)) {
    case 90:
      return { x: viewportY, y: 1 - viewportX };
    case 180:
      return { x: 1 - viewportX, y: 1 - viewportY };
    case 270:
      return { x: 1 - viewportY, y: viewportX };
    default:
      return { x: viewportX, y: viewportY };
  }
}

export function viewportDragToLogicalRect(start, end, viewerRotation = 0) {
  const first = viewportPointToLogicalNormalized(start, viewerRotation);
  const last = viewportPointToLogicalNormalized(end, viewerRotation);
  if (!first || !last) return null;

  const x = Math.min(first.x, last.x);
  const y = Math.min(first.y, last.y);
  const width = Math.abs(last.x - first.x);
  const height = Math.abs(last.y - first.y);
  if (width <= 0 || height <= 0) return null;

  return {
    x: cleanNumber(x),
    y: cleanNumber(y),
    width: cleanNumber(width),
    height: cleanNumber(height),
  };
}

export function logicalRectToViewportRect(rect, viewerRotation = 0) {
  const rotated = rotateNormalizedRectForViewport(rect, normalizedRotation(viewerRotation));
  if (!rotated) return null;
  const { x, y, width, height } = rotated;
  if (
    ![x, y, width, height].every(Number.isFinite) ||
    x < 0 || y < 0 || width <= 0 || height <= 0 ||
    x + width > 1 || y + height > 1
  ) return null;
  return { x, y, width, height };
}
