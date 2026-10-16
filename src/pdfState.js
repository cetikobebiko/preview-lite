import { guardFeatureAction } from "./featureActionGuard.js";
import { getRuntimeFeatureContext } from "./featureRegistry.js";
import * as manipulationBackend from "./services/backend/manipulation.js";
import * as ocrBackend from "./services/backend/ocr.js";
import * as sessionBackend from "./services/backend/session.js";

// ==========================================
// AŞAMA 9: Belge Manipülasyonu
// ==========================================

export async function rotatePage(sessionId, generation, pageIndex, degrees) {
  return await sessionBackend.rotateSessionPage(
    sessionId,
    generation,
    pageIndex,
    degrees,
  );
}

export async function deletePages(sessionId, generation, pageIndices) {
  return await sessionBackend.deleteSessionPages(
    sessionId,
    generation,
    pageIndices,
  );
}

export async function reorderPages(sessionId, generation, newOrder) {
  return await sessionBackend.reorderSessionPages(
    sessionId,
    generation,
    newOrder,
  );
}

export async function insertBlankPage(sessionId, generation, atIndex, width = 595.0, height = 842.0) {
  return await guardFeatureAction(
    "page_insert_blank",
    getRuntimeFeatureContext(),
    () => sessionBackend.insertSessionBlankPage(
      sessionId,
      generation,
      atIndex,
      width,
      height,
    ),
  );
}

export async function mergePdfs(filePaths, outputPath) {
  return await manipulationBackend.mergePdfs(filePaths, outputPath);
}

export async function mergeSessionPdfs(sessionId, expectedContentGeneration, additionalPaths, targetPath) {
  return await manipulationBackend.mergeSessionPdfs(sessionId, expectedContentGeneration, additionalPaths, targetPath);
}

export async function splitSessionPdf(sessionId, expectedContentGeneration, ranges, outputDir) {
  return await manipulationBackend.splitSessionPdf(sessionId, expectedContentGeneration, ranges, outputDir);
}

// ==========================================
// AŞAMA 10: Optik Karakter Tanıma (OCR)
// ==========================================

// Belge başına tek job; sayfa başına bool vektörü döner. Hata yutulmaz:
// cache/fallback politikası ocrDocumentState.js içindedir.
export async function detectScannedPages(sessionId, expectedContentGeneration) {
  return await ocrBackend.detectScannedPages(sessionId, expectedContentGeneration);
}

// Hata yutulmaz; fallback ve yeniden deneme politikası ocrDocumentState.js içindedir.
export async function getAvailableOcrLanguages() {
  return await ocrBackend.getAvailableOcrLanguages();
}

export async function runOcrOnPage(
  sessionId,
  expectedContentGeneration,
  pageIndex,
  lang,
) {
  return await ocrBackend.runOcrOnPage(
    sessionId,
    expectedContentGeneration,
    pageIndex,
    lang,
  );
}

export async function exportAsSearchablePdf(
  sessionId,
  expectedContentGeneration,
  outputPath,
  lang,
) {
  return await guardFeatureAction(
    "searchable_pdf_export",
    getRuntimeFeatureContext(),
    () => ocrBackend.exportAsSearchablePdf(
      sessionId,
      expectedContentGeneration,
      outputPath,
      lang,
    ),
  );
}


export async function getPageAnnotations(
  sessionId,
  generation,
  pageIndex,
  context = getRuntimeFeatureContext(),
) {
  return await guardFeatureAction(
    "annotation",
    context,
    async () => {
      try {
        return await sessionBackend.getSessionPageAnnotations(
          sessionId,
          generation,
          pageIndex,
        );
      } catch (error) {
        console.error("Açıklama okuma hatası:", error);
        return [];
      }
    },
  );
}

export async function addAnnotation(
  sessionId,
  generation,
  pageIndex,
  annotType,
  rect,
  color,
  contents,
  context = getRuntimeFeatureContext(),
) {
  return await guardFeatureAction(
    "annotation",
    context,
    async () => {
      try {
        return await sessionBackend.addSessionAnnotation(
          sessionId,
          generation,
          pageIndex,
          annotType,
          rect,
          color,
          contents,
        );
      } catch (error) {
        console.error("Açıklama ekleme hatası:", error);
        throw error;
      }
    },
  );
}

export async function deleteAnnotation(
  sessionId,
  generation,
  pageIndex,
  annotId,
  context = getRuntimeFeatureContext(),
) {
  return await guardFeatureAction(
    "annotation",
    context,
    async () => {
      try {
        return await sessionBackend.deleteSessionAnnotation(
          sessionId,
          generation,
          pageIndex,
          annotId,
        );
      } catch (error) {
        console.error("Açıklama silme hatası:", error);
        throw error;
      }
    },
  );
}

export async function searchPdfText(sessionId, generation, query) {
  return await sessionBackend.searchSessionPdfText(sessionId, generation, query);
}

import { getConfigRecentFiles } from "./configManager.js";

export async function getRecentFiles() {
  return getConfigRecentFiles();
}

export function getBookSpreads(pageCount, coverMode = true) {
  const pageOrder = Array.from({ length: pageCount }, (_, i) => i);
  const spreads = [];
  let i = 0;
  if (coverMode && pageOrder.length > 0) {
    spreads.push([pageOrder[0]]);
    i = 1;
  }
  while (i < pageOrder.length) {
    if (i + 1 < pageOrder.length) {
      spreads.push([pageOrder[i], pageOrder[i + 1]]);
      i += 2;
    } else {
      spreads.push([pageOrder[i]]);
      i += 1;
    }
  }
  return spreads;
}
