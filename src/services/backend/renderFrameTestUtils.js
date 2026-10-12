import {
  RENDER_FRAME_FIXED_HEADER_BYTES,
  RENDER_FRAME_MAGIC,
  RENDER_FRAME_VERSION,
} from "./renderFrame.js";

/** Test-only mirror of the Rust encoder; builds a v1 render frame ArrayBuffer. */
export function encodeRenderFrame({
  width,
  height,
  data,
  pageIndex,
  generation,
  requestId,
}) {
  const requestIdBytes = new TextEncoder().encode(requestId);
  const pixelOffset = RENDER_FRAME_FIXED_HEADER_BYTES + requestIdBytes.length;
  const buffer = new ArrayBuffer(pixelOffset + data.length);
  const bytes = new Uint8Array(buffer);
  const view = new DataView(buffer);
  bytes.set(RENDER_FRAME_MAGIC, 0);
  view.setUint16(4, RENDER_FRAME_VERSION, true);
  view.setUint32(6, width, true);
  view.setUint32(10, height, true);
  view.setUint32(14, pageIndex, true);
  view.setBigUint64(18, BigInt(generation), true);
  view.setUint16(26, requestIdBytes.length, true);
  bytes.set(requestIdBytes, RENDER_FRAME_FIXED_HEADER_BYTES);
  bytes.set(data, pixelOffset);
  return buffer;
}
