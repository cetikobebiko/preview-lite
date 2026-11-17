import { describe, expect, it } from "vitest";
import {
  preferredOcrLanguage,
  searchablePdfDetails,
  searchablePdfErrorMessage,
  searchablePdfSummary,
  selectableOcrLanguages,
} from "./searchablePdf.js";

const error = (code, message = "mesaj") => Object.assign(new Error(message), { code });

describe("searchable PDF helpers", () => {
  it("prefers Turkish, then English, and never offers osd", () => {
    expect(preferredOcrLanguage(["eng", "osd", "tur"])).toBe("tur");
    expect(preferredOcrLanguage(["deu", "eng"])).toBe("eng");
    expect(preferredOcrLanguage(["osd", "fra"])).toBe("fra");
    expect(preferredOcrLanguage([])).toBe("");
    expect(selectableOcrLanguages(["osd", "tur", "eng"])).toEqual(["tur", "eng"]);
  });

  it("summarizes processed and skipped pages with 1-based page numbers", () => {
    const report = {
      processedPages: [0, 1, 3],
      skippedPages: [
        { pageIndex: 2, reason: "ocr_failed", detail: "bozuk" },
        { pageIndex: 4, reason: "has_text", detail: null },
        { pageIndex: 6, reason: "has_text", detail: null },
        { pageIndex: 5, reason: "no_image", detail: null },
      ],
      wordsWritten: 120,
      wordsSkipped: 2,
      signaturesInvalidated: true,
    };
    expect(searchablePdfSummary(report)).toBe(
      "Aranabilir kopya kaydedildi: 3 sayfaya metin katmanı eklendi (120 kelime). 4 sayfa atlandı.",
    );
    expect(searchablePdfDetails(report)).toEqual([
      "Atlandı, taranmış görüntü yok: 6",
      "Atlandı, zaten metin katmanı var: 5, 7",
      "Sayfa 3: tanınamadı (bozuk)",
      "2 kelime yazılamadı (boş, desteklenmeyen karakter ya da sayfa dışı).",
      "Belgedeki dijital imzalar kopyada geçersiz.",
    ]);
  });

  it("turns missing Tesseract or language data into install guidance", () => {
    expect(searchablePdfErrorMessage(error("OCR_ENGINE_NOT_FOUND"))).toMatch(/Tesseract bulunamadı.*brew install tesseract/);
    expect(searchablePdfErrorMessage(error("OCR_LANGUAGE_MISSING", "Tesseract dil verisi kurulu değil: tur."))).toMatch(
      /kurulu değil: tur\. Kurulum:.*tesseract-ocr-tur/,
    );
    expect(searchablePdfErrorMessage(error("SESSION_JOB_CANCELLED"))).toBe("İşlem iptal edildi; hiçbir dosya yazılmadı.");
    expect(searchablePdfErrorMessage(error("SEARCHABLE_PDF_ENCRYPTED", "Parola korumalı"))).toBe("Parola korumalı");
  });
});
