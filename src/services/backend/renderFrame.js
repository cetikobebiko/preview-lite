// Binary render frame decoder for `render_session_page` / `render_session_thumbnail`.
// Wire format v1 (all integers little-endian), encoded by
// `src-tauri/src/pdf_engine/render_wire.rs`:
//   offset  size  field
//   0       4     magic "AGRF"
//   4       2     version (u16) = 1
//   6       4     width (u32)
//   10      4     height (u32)
//   14      4     pageIndex (u32)
//   18      8     generation (u64, <= Number.MAX_SAFE_INTEGER)
//   26      2     requestId byte length (u16, 1..=512; <= 128 chars)
//   28      n     requestId (UTF-8)
//   28+n    w*h*4 RGBA pixels
// Errors never use this format; they stay JSON (`DocumentSessionError`).

export const RENDER_FRAME_MAGIC = Object.freeze([0x41, 0x47, 0x52, 0x46]);
export const RENDER_FRAME_VERSION = 1;
export const RENDER_FRAME_FIXED_HEADER_BYTES = 28;
// Request IDs are <= 128 characters; a UTF-8 character is at most 4 bytes.
const MAX_REQUEST_ID_BYTES = 512;

const requestIdDecoder = new TextDecoder("utf-8", { fatal: true });

function invalidFrame() {
  return new Error("render_invalid_rgba_payload");
}

function toBytes(raw) {
  if (raw instanceof ArrayBuffer) return new Uint8Array(raw);
  if (raw instanceof Uint8Array) return raw;
  // Tauri's postMessage IPC fallback delivers raw bodies as a number array.
  if (Array.isArray(raw)) return Uint8Array.from(raw);
  throw invalidFrame();
}

/** Decodes a v1 render frame; pixel data is a zero-copy view over the IPC buffer. */
export function decodeRenderFrame(raw) {
  const bytes = toBytes(raw);
  if (bytes.byteLength < RENDER_FRAME_FIXED_HEADER_BYTES) throw invalidFrame();
  for (let index = 0; index < RENDER_FRAME_MAGIC.length; index += 1) {
    if (bytes[index] !== RENDER_FRAME_MAGIC[index]) throw invalidFrame();
  }

  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint16(4, true) !== RENDER_FRAME_VERSION) throw invalidFrame();
  const width = view.getUint32(6, true);
  const height = view.getUint32(10, true);
  const pageIndex = view.getUint32(14, true);
  const generation = view.getBigUint64(18, true);
  const requestIdLength = view.getUint16(26, true);

  if (
    width === 0 ||
    height === 0 ||
    generation > BigInt(Number.MAX_SAFE_INTEGER) ||
    requestIdLength === 0 ||
    requestIdLength > MAX_REQUEST_ID_BYTES
  ) {
    throw invalidFrame();
  }

  const pixelOffset = RENDER_FRAME_FIXED_HEADER_BYTES + requestIdLength;
  const pixelLength = width * height * 4;
  if (
    !Number.isSafeInteger(pixelLength) ||
    bytes.byteLength !== pixelOffset + pixelLength
  ) {
    throw invalidFrame();
  }

  let requestId;
  try {
    requestId = requestIdDecoder.decode(
      bytes.subarray(RENDER_FRAME_FIXED_HEADER_BYTES, pixelOffset),
    );
  } catch {
    throw invalidFrame();
  }

  return {
    width,
    height,
    data: new Uint8ClampedArray(
      bytes.buffer,
      bytes.byteOffset + pixelOffset,
      pixelLength,
    ),
    pageIndex,
    generation: Number(generation),
    requestId,
  };
}
