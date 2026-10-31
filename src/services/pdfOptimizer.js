// Mirrors `pdf_optimizer::OptimizeSettings` and `validate_request`, so the dialog refuses what
// the backend would.

export const MIN_PPI = 9;
export const MAX_PPI = 2400;

const CLEAN_UP = Object.freeze({
  removeUnusedXobjects: true,
  discardThumbnails: true,
  discardAlternateImages: true,
  flateUnencoded: true,
  objectStreams: true,
});

const images = (targetPpi, abovePpi, quality) =>
  Object.freeze({ downsample: true, targetPpi, abovePpi, compression: Object.freeze({ kind: "jpeg", quality }) });

const preset = (id, label, description, imageSettings) =>
  Object.freeze({
    id,
    label,
    description,
    settings: Object.freeze({ color: imageSettings, gray: imageSettings, ...CLEAN_UP }),
  });

// Acrobat's Reduce File Size choices (PdfCraft's presets), plus print and lossless ones.
export const OPTIMIZE_PRESETS = Object.freeze([
  preset("standard", "Standart", "Görüntüler 225 ppi üstündeyse 150 ppi'ye, JPEG orta kalite.", images(150, 225, 60)),
  preset("screen", "Ekran ve mobil", "Görüntüler 144 ppi üstündeyse 96 ppi'ye, JPEG yüksek kalite.", images(96, 144, 80)),
  preset("print", "Yüksek kalite (baskı)", "Görüntüler 450 ppi üstündeyse 300 ppi'ye, JPEG yüksek kalite.", images(300, 450, 80)),
  preset(
    "lossless",
    "Kayıpsız",
    "Görüntülere dokunulmaz; yalnız kullanılmayan nesneler atılır ve akışlar sıkıştırılır.",
    Object.freeze({ downsample: false, targetPpi: 150, abovePpi: 225, compression: Object.freeze({ kind: "retain" }) }),
  ),
]);

export const JPEG_QUALITIES = Object.freeze([
  Object.freeze({ value: 25, label: "En düşük" }),
  Object.freeze({ value: 45, label: "Düşük" }),
  Object.freeze({ value: 60, label: "Orta" }),
  Object.freeze({ value: 80, label: "Yüksek" }),
  Object.freeze({ value: 95, label: "En yüksek" }),
]);

/** A mutable copy of a preset's settings, for the dialog to edit. */
export function presetSettings(id) {
  const found = OPTIMIZE_PRESETS.find((candidate) => candidate.id === id) ?? OPTIMIZE_PRESETS[0];
  // Through JSON, not `structuredClone`: a preset's colour and grey settings are one object, and
  // the copy must let the dialog edit them apart.
  return JSON.parse(JSON.stringify(found.settings));
}

function imageSettingsError(settings, what) {
  const inRange = (value) => Number.isFinite(value) && value >= MIN_PPI && value <= MAX_PPI;
  if (!inRange(settings.targetPpi) || !inRange(settings.abovePpi)) {
    return `${what} görüntü çözünürlükleri ${MIN_PPI}–${MAX_PPI} ppi aralığında olmalı.`;
  }
  if (settings.abovePpi < settings.targetPpi) {
    return `${what} görüntülerde eşik çözünürlüğü hedef çözünürlükten küçük olamaz.`;
  }
  const { kind, quality } = settings.compression ?? {};
  if (kind === "jpeg" && !(Number.isInteger(quality) && quality >= 1 && quality <= 100)) {
    return `${what} görüntülerde JPEG kalitesi 1–100 olmalı.`;
  }
  if (!["jpeg", "flate", "retain"].includes(kind)) return `${what} görüntüler için sıkıştırma seçilmedi.`;
  return null;
}

/** The first problem with `settings`, or null. */
export function optimizeSettingsError(settings) {
  return imageSettingsError(settings.color, "Renkli") ?? imageSettingsError(settings.gray, "Gri tonlamalı");
}

export function formatBytes(bytes) {
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toLocaleString("tr-TR", { maximumFractionDigits: digits, minimumFractionDigits: digits })} ${units[unit]}`;
}

/** One-line result for a toast: sizes and the saving (or growth). */
export function optimizeSummary(report) {
  const sizes = `${formatBytes(report.inputBytes)} → ${formatBytes(report.outputBytes)}`;
  if (!report.inputBytes) return `Optimize edilmiş kopya kaydedildi (${sizes}).`;
  const change = Math.round((1 - report.outputBytes / report.inputBytes) * 100);
  if (change > 0) return `Optimize edilmiş kopya kaydedildi: ${sizes} (%${change} daha küçük).`;
  return `Kopya kaydedildi ama küçülmedi: ${sizes}.`;
}

/** The report's details, one line each, for the dialog. */
export function optimizeDetails(report) {
  const lines = [];
  const changed = report.imagesResampled + report.imagesRecompressed;
  if (report.images > 0 && changed === 0) {
    lines.push(`${report.images} görüntü değiştirilmedi: daha küçük olmayacaktı ya da biçimleri desteklenmiyor.`);
  } else if (changed > 0) {
    lines.push(
      `${report.images} görüntüden ${changed} tanesi küçültüldü (${report.imagesResampled} yeniden örneklendi, ${report.imagesRecompressed} yeniden sıkıştırıldı): ${formatBytes(report.imageBytesBefore)} → ${formatBytes(report.imageBytesAfter)}.`,
    );
  }
  if (report.unusedXobjects > 0) lines.push(`${report.unusedXobjects} kullanılmayan görüntü/form atıldı.`);
  if (report.thumbnails > 0) lines.push(`${report.thumbnails} sayfa küçük resmi atıldı.`);
  if (report.alternateImages > 0) lines.push(`${report.alternateImages} görüntünün alternatifleri atıldı.`);
  if (report.streamsCompressed > 0) lines.push(`${report.streamsCompressed} sıkıştırılmamış akış sıkıştırıldı.`);
  if (report.signaturesInvalidated) lines.push("Belgedeki dijital imzalar kopyada geçersizdir.");
  return lines;
}
