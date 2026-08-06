import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { paintBufferToCanvas, transformRgbaBuffer } from "./canvasUtils.js";

const originalImageData = globalThis.ImageData;

function createCanvasMock() {
  const context = {
    fillStyle: "",
    fillRect: vi.fn(),
    putImageData: vi.fn(),
  };
  const canvas = {
    width: 0,
    height: 0,
    getContext: vi.fn(() => context),
  };
  return { canvas, context };
}

beforeEach(() => {
  globalThis.ImageData = class ImageDataMock {
    constructor(data, width, height) {
      this.data = data;
      this.width = width;
      this.height = height;
    }
  };
});

afterEach(() => {
  globalThis.ImageData = originalImageData;
});

describe("paintBufferToCanvas", () => {
  it.each([
    [[10, 20, 30, 255]],
    [new Uint8Array([10, 20, 30, 255])],
  ])("normal dizi ve Uint8Array tamponlarını çizer", (data) => {
    const { canvas, context } = createCanvasMock();

    paintBufferToCanvas(canvas, { width: 1, height: 1, data });

    expect(canvas.width).toBe(1);
    expect(canvas.height).toBe(1);
    expect(context.fillRect).toHaveBeenCalledWith(0, 0, 1, 1);
    expect(context.putImageData).toHaveBeenCalledOnce();
    expect(Array.from(context.putImageData.mock.calls[0][0].data)).toEqual([10, 20, 30, 255]);
  });

  it.each([
    [{ width: 0, height: 1, data: [] }],
    [{ width: -1, height: 1, data: [] }],
    [{ width: Number.NaN, height: 1, data: [] }],
    [{ width: 1, height: Number.POSITIVE_INFINITY, data: [] }],
  ])("geçersiz boyutları reddeder", (buffer) => {
    const { canvas } = createCanvasMock();
    expect(() => paintBufferToCanvas(canvas, buffer)).toThrow("Geçersiz görüntü verisi.");
  });

  it.each([
    [[0, 0, 0, 255]],
    [[0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]],
  ])("tam RGBA uzunluğuyla eşleşmeyen tamponu reddeder", (data) => {
    const { canvas } = createCanvasMock();
    expect(() =>
      paintBufferToCanvas(canvas, { width: 2, height: 1, data }),
    ).toThrow("Eksik görüntü piksel verisi.");
  });
});

describe("transformRgbaBuffer", () => {
  it("kaynak tamponu değiştirmeden Rust ile eşleşen renk dönüşümlerini uygular", () => {
    const source = new Uint8Array([10, 20, 30, 40]);

    expect(Array.from(transformRgbaBuffer(source, "dark"))).toEqual([245, 235, 225, 40]);
    expect(Array.from(transformRgbaBuffer(source, "grayscale"))).toEqual([18, 18, 18, 40]);
    expect(Array.from(transformRgbaBuffer(source, "sepia"))).toEqual([24, 22, 17, 40]);
    expect(Array.from(source)).toEqual([10, 20, 30, 40]);
  });

  it("bilinmeyen renk modunda güvenli biçimde normal veriyi kullanır", () => {
    const source = [1, 2, 3, 4];
    expect(Array.from(transformRgbaBuffer(source, "unknown"))).toEqual(source);
  });
});
