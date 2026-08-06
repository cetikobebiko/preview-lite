import { describe, expect, it, vi } from "vitest";
import {
  createTextLayerItems,
  normalizeTextRotation,
  positionTextBox,
  sanitizeTextLayout,
} from "./textLayerUtils.js";

const BOX = { x: 0.1, y: 0.2, width: 0.3, height: 0.4, text: "word" };
const measureThirty = () => 30;

describe("normalizeTextRotation", () => {
  it.each([
    [0, 0],
    [90, 90],
    [180, 180],
    [270, 270],
    [-90, 270],
    [450, 90],
    [45, 0],
  ])("%s derecesini %s olarak normalize eder", (input, expected) => {
    expect(normalizeTextRotation(input)).toBe(expected);
  });
});

describe("positionTextBox", () => {
  it.each([
    [0, 200, 100, { left: 20, top: 20, visualWidth: 60, visualHeight: 40 }],
    [90, 100, 200, { left: 40, top: 20, visualWidth: 40, visualHeight: 60 }],
    [180, 200, 100, { left: 120, top: 40, visualWidth: 60, visualHeight: 40 }],
    [270, 100, 200, { left: 20, top: 120, visualWidth: 40, visualHeight: 60 }],
  ])(
    "%s° için normalize kutuyu nihai CSS yüzeyine dönüştürür",
    (rotation, pageCssWidth, pageCssHeight, expected) => {
      const result = positionTextBox(
        BOX,
        rotation,
        pageCssWidth,
        pageCssHeight,
        measureThirty,
      );

      expect(result.left).toBeCloseTo(expected.left);
      expect(result.top).toBeCloseTo(expected.top);
      expect(result.visualWidth).toBeCloseTo(expected.visualWidth);
      expect(result.visualHeight).toBeCloseTo(expected.visualHeight);
      expect(result.scaleX).toBeCloseTo(2);
      expect(result.rotation).toBe(rotation);
    },
  );

  it("sayfa dışına taşan kutuyu güvenli biçimde kırpar", () => {
    expect(
      sanitizeTextLayout({
        x: -0.1,
        y: 0.9,
        width: 0.3,
        height: 0.3,
        text: " clipped ",
      }),
    ).toEqual({
      x: 0,
      y: 0.9,
      width: 0.19999999999999998,
      height: 0.09999999999999998,
      text: "clipped",
    });
  });

  it.each([
    [{ ...BOX, width: 0 }],
    [{ ...BOX, height: -1 }],
    [{ ...BOX, x: Number.NaN }],
    [{ ...BOX, y: Number.POSITIVE_INFINITY }],
    [{ ...BOX, text: "   " }],
    [null],
  ])("bozuk veya boş yerleşimi reddeder", (layout) => {
    expect(positionTextBox(layout, 0, 200, 100, measureThirty)).toBeNull();
  });

  it("ölçüm hatasında sınırlı bir fallback scaleX üretir", () => {
    const result = positionTextBox(BOX, 0, 200, 100, () => {
      throw new Error("canvas unavailable");
    });

    expect(result.scaleX).toBeGreaterThanOrEqual(0.05);
    expect(result.scaleX).toBeLessThanOrEqual(20);
  });
});

describe("createTextLayerItems", () => {
  it("kaynak sırasını korur, boşluğu ölçüme katmaz ve yalnızca aradaki spanlara ekler", () => {
    const measure = vi.fn(() => 20);
    const source = [
      { x: 0.1, y: 0.1, width: 0.2, height: 0.1, text: "<first&>" },
      { x: 0.4, y: 0.1, width: 0.2, height: 0.1, text: "second" },
    ];
    const snapshot = structuredClone(source);

    const items = createTextLayerItems(source, {
      pageCssWidth: 200,
      pageCssHeight: 100,
      measureTextWidth: measure,
    });

    expect(items.map((item) => item.text)).toEqual(["<first&>", "second"]);
    expect(items.map((item) => item.hasTrailingSpace)).toEqual([true, false]);
    expect(measure.mock.calls.map(([text]) => text)).toEqual(["<first&>", "second"]);
    expect(source).toEqual(snapshot);
  });

  it("geçersiz öğeleri atıp kalan öğelerin kontrollü boşluk durumunu yeniden hesaplar", () => {
    const items = createTextLayerItems(
      [
        { ...BOX, text: "" },
        { ...BOX, text: "kept" },
        { ...BOX, width: 0 },
      ],
      {
        pageCssWidth: 200,
        pageCssHeight: 100,
        measureTextWidth: measureThirty,
      },
    );

    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ text: "kept", hasTrailingSpace: false });
  });
});
