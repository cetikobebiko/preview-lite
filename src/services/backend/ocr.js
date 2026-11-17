import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";

export const detectScannedPages = (sessionId, expectedContentGeneration) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("detectScannedPages", { jobId, sessionId, expectedContentGeneration }),
  ).promise;

export const getAvailableOcrLanguages = () =>
  callBackend("getAvailableOcrLanguages");

export const runOcrOnPage = (sessionId, expectedContentGeneration, pageIndex, lang) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("runOcrOnPage", { jobId, sessionId, expectedContentGeneration, pageIndex, lang }),
  ).promise;

// Uzun süren, iptal edilebilir job: `{ jobId, promise, cancel }` döner.
// `pages` null ise bütün sayfalar değerlendirilir (metni olanlar her durumda atlanır).
export const startSearchablePdfExport = (sessionId, expectedContentGeneration, outputPath, lang, pages = null) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("exportAsSearchablePdf", { jobId, sessionId, expectedContentGeneration, outputPath, lang, pages }),
  );
