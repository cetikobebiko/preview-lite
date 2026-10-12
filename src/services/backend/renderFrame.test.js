import { describe, expect, it } from "vitest";
import {
  RENDER_FRAME_FIXED_HEADER_BYTES,
  decodeRenderFrame,
} from "./renderFrame.js";
import { encodeRenderFrame } from "./renderFrameTestUtils.js";

// Same bytes are asserted by the Rust encoder test
// (`render_wire::tests::encodes_golden_frame`).
const GOLDEN_FRAME = [
  0x41, 0x47, 0x52, 0x46,
  0x01, 0x00,
  0x01, 0x00, 0x00, 0x00,
  0x01, 0x00, 0x00, 0x00,
  0x02, 0x00, 0x00, 0x00,
  0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
  0x02, 0x00,
  0x72, 0x31,
  0x0a, 0x14, 0x1e, 0xff,
];

const frame = (overrides = {}) =>
  encodeRenderFrame({
    width: 2,
    height: 1,
    data: new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]),
    pageIndex: 3,
    generation: 9,
    requestId: "main_page-request-1",
    ...overrides,
  });

describe("decodeRenderFrame", () => {
  it("decodes the cross-language golden frame", () => {
    const decoded = decodeRenderFrame(new Uint8Array(GOLDEN_FRAME).buffer);

    expect(decoded).toMatchObject({
      width: 1,
      height: 1,
      pageIndex: 2,
      generation: 7,
      requestId: "r1",
    });
    expect(Array.from(decoded.data)).toEqual([10, 20, 30, 255]);
  });

  it("returns pixel data as a zero-copy Uint8ClampedArray view", () => {
    const buffer = frame();
    const decoded = decodeRenderFrame(buffer);

    expect(decoded.data).toBeInstanceOf(Uint8ClampedArray);
    expect(decoded.data.buffer).toBe(buffer);
    expect(decoded.data.byteOffset).toBe(
      RENDER_FRAME_FIXED_HEADER_BYTES + "main_page-request-1".length,
    );
    expect(Array.from(decoded.data)).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
  });

  it("round-trips the maximum safe generation and a multi-byte request ID", () => {
    const decoded = decodeRenderFrame(
      frame({ generation: Number.MAX_SAFE_INTEGER, requestId: "istek-ğüş" }),
    );
    expect(decoded.generation).toBe(Number.MAX_SAFE_INTEGER);
    expect(decoded.requestId).toBe("istek-ğüş");
  });

  it("accepts Tauri's postMessage fallback number-array body", () => {
    const decoded = decodeRenderFrame(GOLDEN_FRAME);
    expect(Array.from(decoded.data)).toEqual([10, 20, 30, 255]);
  });

  it.each([
    ["empty buffer", () => new ArrayBuffer(0)],
    ["short header", () => frame().slice(0, RENDER_FRAME_FIXED_HEADER_BYTES - 1)],
    ["truncated pixels", () => frame().slice(0, -1)],
    ["trailing bytes", () => {
      const source = new Uint8Array(frame());
      const extended = new Uint8Array(source.length + 1);
      extended.set(source);
      return extended.buffer;
    }],
    ["wrong magic", () => {
      const bytes = new Uint8Array(frame());
      bytes[0] = 0x00;
      return bytes.buffer;
    }],
    ["unsupported version", () => {
      const buffer = frame();
      new DataView(buffer).setUint16(4, 2, true);
      return buffer;
    }],
    ["width/height mismatch with pixel length", () => {
      const buffer = frame();
      new DataView(buffer).setUint32(6, 3, true);
      return buffer;
    }],
    ["zero width", () => frame({ width: 0, data: new Uint8Array() })],
    ["generation above MAX_SAFE_INTEGER", () => frame({ generation: 2 ** 53 })],
    ["empty request ID", () => frame({ requestId: "" })],
    ["request ID longer than 512 bytes", () => frame({ requestId: "r".repeat(513) })],
    ["invalid UTF-8 request ID", () => {
      const buffer = frame({ requestId: "ab" });
      new Uint8Array(buffer)[RENDER_FRAME_FIXED_HEADER_BYTES] = 0xff;
      return buffer;
    }],
    ["JSON object body", () => ({ width: 1, height: 1, data: [0, 0, 0, 0] })],
    ["null body", () => null],
  ])("rejects %s through the existing invalid payload error", (_label, build) => {
    expect(() => decodeRenderFrame(build())).toThrow("render_invalid_rgba_payload");
  });
});
