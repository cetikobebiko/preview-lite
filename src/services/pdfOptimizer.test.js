import { describe, expect, it } from "vitest";
import {
  OPTIMIZE_PRESETS,
  formatBytes,
  optimizeDetails,
  optimizeSettingsError,
  optimizeSummary,
  presetSettings,
} from "./pdfOptimizer.js";

const report = (overrides = {}) => ({
  inputBytes: 10 * 1024 * 1024,
  outputBytes: 2.5 * 1024 * 1024,
  images: 4,
  imagesResampled: 2,
  imagesRecompressed: 1,
  imageBytesBefore: 9 * 1024 * 1024,
  imageBytesAfter: 1.5 * 1024 * 1024,
  unusedXobjects: 0,
  thumbnails: 0,
  alternateImages: 0,
  streamsCompressed: 3,
  signaturesInvalidated: false,
  ...overrides,
});

describe("optimizer settings", () => {
  it("offers valid presets as independent copies", () => {
    for (const preset of OPTIMIZE_PRESETS) {
      expect(optimizeSettingsError(presetSettings(preset.id))).toBeNull();
    }
    const copy = presetSettings("standard");
    copy.color.targetPpi = 72;
    expect(presetSettings("standard").color.targetPpi).toBe(150);
    // Colour and grey are one object in a preset; a copy edits them apart.
    expect(copy.gray.targetPpi).toBe(150);
    expect(presetSettings("unknown")).toEqual(presetSettings("standard"));
    expect(presetSettings("lossless").color).toMatchObject({ downsample: false, compression: { kind: "retain" } });
  });

  it("refuses what the backend refuses", () => {
    const settings = presetSettings("standard");
    settings.color.targetPpi = 5;
    expect(optimizeSettingsError(settings)).toMatch(/Renkli.*9–2400/);
    const inverted = presetSettings("standard");
    inverted.gray.abovePpi = 100;
    expect(optimizeSettingsError(inverted)).toMatch(/Gri tonlamalı.*eşik/);
    const quality = presetSettings("standard");
    quality.color.compression = { kind: "jpeg", quality: 0 };
    expect(optimizeSettingsError(quality)).toMatch(/JPEG kalitesi/);
    const empty = presetSettings("standard");
    empty.color.targetPpi = null;
    expect(optimizeSettingsError(empty)).toMatch(/aralığında/);
  });
});

describe("optimizer report", () => {
  it("formats sizes in Turkish", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1,5 KB");
    expect(formatBytes(250 * 1024 * 1024)).toBe("250 MB");
  });

  it("states the saving, or that the copy did not shrink", () => {
    expect(optimizeSummary(report())).toBe("Optimize edilmiş kopya kaydedildi: 10,0 MB → 2,5 MB (%75 daha küçük).");
    expect(optimizeSummary(report({ outputBytes: 11 * 1024 * 1024 }))).toMatch(/küçülmedi/);
  });

  it("lists only what changed, and warns about signatures", () => {
    expect(optimizeDetails(report())).toEqual([
      "4 görüntüden 3 tanesi küçültüldü (2 yeniden örneklendi, 1 yeniden sıkıştırıldı): 9,0 MB → 1,5 MB.",
      "3 sıkıştırılmamış akış sıkıştırıldı.",
    ]);
    expect(optimizeDetails(report({ imagesResampled: 0, imagesRecompressed: 0, streamsCompressed: 0 }))).toEqual([
      "4 görüntü değiştirilmedi: daha küçük olmayacaktı ya da biçimleri desteklenmiyor.",
    ]);
    expect(optimizeDetails(report({ images: 0, imagesResampled: 0, imagesRecompressed: 0, streamsCompressed: 0, unusedXobjects: 2, signaturesInvalidated: true }))).toEqual([
      "2 kullanılmayan görüntü/form atıldı.",
      "Belgedeki dijital imzalar kopyada geçersizdir.",
    ]);
  });
});
