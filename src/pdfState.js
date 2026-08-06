import { guardFeatureAction } from "./featureActionGuard.js";
import { getRuntimeFeatureContext } from "./featureRegistry.js";
import * as aiBackend from "./services/backend/ai.js";
import { burnAnnotationsToPdf as burnAnnotationsToPdfBackend } from "./services/backend/document.js";
import * as manipulationBackend from "./services/backend/manipulation.js";
import * as ocrBackend from "./services/backend/ocr.js";
import * as sessionBackend from "./services/backend/session.js";

export async function burnAnnotationsToPdf(
  sessionId,
  expectedContentGeneration,
  annotationsJson,
  targetPath,
) {
  return await guardFeatureAction(
    "annotation_burn_export",
    getRuntimeFeatureContext(),
    async () => {
      await burnAnnotationsToPdfBackend(
        sessionId,
        expectedContentGeneration,
        annotationsJson,
        targetPath,
      );
      return true;
    },
  );
}

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

export async function detectScannedPage(
  sessionId,
  expectedContentGeneration,
  pageIndex,
) {
  try {
    return await ocrBackend.detectScannedPage(
      sessionId,
      expectedContentGeneration,
      pageIndex,
    );
  } catch (e) {
    console.error("Scanned page tespiti hatası:", e);
    return false; // Hata durumunda varsayılan olarak metin katmanı var sayılır.
  }
}

export async function getAvailableOcrLanguages() {
  try {
    return await ocrBackend.getAvailableOcrLanguages();
  } catch (e) {
    console.error("OCR dilleri okunamadı:", e);
    return ["eng"]; // Fallback
  }
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

// ==========================================
// AŞAMA 11: Semantik Arama ve Yapay Zeka (Local LLM)
// ==========================================

export async function buildSemanticIndex(sessionId, expectedContentGeneration, chunkSize = 500, chunkOverlap = 50, ollamaUrl = "http://127.0.0.1:11434", embedModel = "nomic-embed-text") {
  return await aiBackend.buildSemanticIndex(sessionId, expectedContentGeneration, chunkSize, chunkOverlap, ollamaUrl, embedModel);
}

export async function semanticSearch(sessionId, expectedContentGeneration, query, topK = 3, ollamaUrl = "http://127.0.0.1:11434", embedModel = "nomic-embed-text") {
  return await aiBackend.semanticSearch(sessionId, expectedContentGeneration, query, topK, ollamaUrl, embedModel);
}

export async function queryDocumentRag(sessionId, expectedContentGeneration, prompt, history, ollamaUrl = "http://127.0.0.1:11434", llmModel = "llama3", embedModel = "nomic-embed-text") {
  return await aiBackend.queryDocumentRag(sessionId, expectedContentGeneration, prompt, history, ollamaUrl, llmModel, embedModel);
}

export async function summarizeSelectionOrPage(textContent, targetLength = "short", ollamaUrl = "http://127.0.0.1:11434", llmModel = "llama3") {
  return await aiBackend.summarizeSelectionOrPage(textContent, targetLength, ollamaUrl, llmModel);
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
