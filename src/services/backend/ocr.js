import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";

export const detectScannedPage = (sessionId, expectedContentGeneration, pageIndex) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("detectScannedPage", { jobId, sessionId, expectedContentGeneration, pageIndex }),
  ).promise;

export const getAvailableOcrLanguages = () =>
  callBackend("getAvailableOcrLanguages");

export const runOcrOnPage = (sessionId, expectedContentGeneration, pageIndex, lang) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("runOcrOnPage", { jobId, sessionId, expectedContentGeneration, pageIndex, lang }),
  ).promise;

export const exportAsSearchablePdf = (sessionId, expectedContentGeneration, outputPath, lang) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("exportAsSearchablePdf", { jobId, sessionId, expectedContentGeneration, outputPath, lang }),
  ).promise;
