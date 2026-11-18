// Aranabilir PDF (OCR metin katmanı) dışa aktarma: dil seçimi, rapor özeti ve hata metinleri.

const SKIP_LABELS = Object.freeze({
  has_text: "zaten metin katmanı var",
  no_image: "taranmış görüntü yok",
  no_words: "tanınan metin yok",
  ocr_failed: "tanınamadı",
});

export const TESSERACT_INSTALL_HINT =
  "Kurulum: macOS'ta `brew install tesseract tesseract-lang`; Windows'ta UB-Mannheim Tesseract kurulumu (Türkçe dil verisiyle); Linux'ta `tesseract-ocr` ve `tesseract-ocr-tur` paketleri. Kurduktan sonra uygulamayı yeniden başlatın.";

/** Türkçe varsa onu, yoksa İngilizceyi, yoksa ilk kurulu dili seçer. */
export function preferredOcrLanguage(languages) {
  if (!Array.isArray(languages) || languages.length === 0) return "";
  if (languages.includes("tur")) return "tur";
  if (languages.includes("eng")) return "eng";
  return languages.find((lang) => lang !== "osd") ?? languages[0];
}

/** Seçilebilir diller: `osd` (yalnız yön tespiti) metin tanımaz. */
export function selectableOcrLanguages(languages) {
  return Array.isArray(languages) ? languages.filter((lang) => lang !== "osd") : [];
}

function pageList(pages) {
  return pages.map((page) => page + 1).join(", ");
}

export function searchablePdfSummary(report) {
  const processed = report?.processedPages?.length ?? 0;
  const skipped = report?.skippedPages?.length ?? 0;
  const words = report?.wordsWritten ?? 0;
  let text = `Aranabilir kopya kaydedildi: ${processed} sayfaya metin katmanı eklendi (${words} kelime).`;
  if (skipped > 0) text += ` ${skipped} sayfa atlandı.`;
  return text;
}

/** Atlanan sayfalar, nedene göre gruplanmış satırlar (1 tabanlı sayfa numaralarıyla). */
export function searchablePdfDetails(report) {
  const lines = [];
  const groups = new Map();
  for (const page of report?.skippedPages ?? []) {
    if (page.reason === "ocr_failed") {
      lines.push(`Sayfa ${page.pageIndex + 1}: ${SKIP_LABELS.ocr_failed}${page.detail ? ` (${page.detail})` : ""}`);
      continue;
    }
    if (!groups.has(page.reason)) groups.set(page.reason, []);
    groups.get(page.reason).push(page.pageIndex);
  }
  for (const [reason, pages] of groups) {
    lines.unshift(`Atlandı, ${SKIP_LABELS[reason] ?? reason}: ${pageList(pages)}`);
  }
  if (report?.wordsSkipped > 0) lines.push(`${report.wordsSkipped} kelime yazılamadı (boş, desteklenmeyen karakter ya da sayfa dışı).`);
  if (report?.signaturesInvalidated) lines.push("Belgedeki dijital imzalar kopyada geçersiz.");
  return lines;
}

/** Backend hatasını kullanıcıya gösterilecek, eyleme geçirilebilir metne çevirir. */
export function searchablePdfErrorMessage(error) {
  const code = error?.code ?? "";
  const message = error?.message ?? String(error ?? "");
  if (code === "SESSION_JOB_CANCELLED") return "İşlem iptal edildi; hiçbir dosya yazılmadı.";
  if (code === "OCR_ENGINE_NOT_FOUND" || code === "SESSION_JOB_CHILD_PROGRAM_NOT_FOUND") {
    return `Tesseract bulunamadı. ${TESSERACT_INSTALL_HINT}`;
  }
  if (code === "OCR_LANGUAGE_MISSING") return `${message} ${TESSERACT_INSTALL_HINT}`;
  if (code === "SEARCHABLE_PDF_NO_SCANNED_PAGES") {
    return `${message}. Metni olan sayfalara ikinci bir metin katmanı eklenmez.`;
  }
  return message;
}
