import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";

export const speakText = (text, voice) =>
  callBackend("speakText", { text, voice });

export const stopSpeaking = () => callBackend("stopSpeaking");

export const readAppConfig = () => callBackend("readAppConfig");

export const writeAppConfig = (contents) =>
  callBackend("writeAppConfig", { contents });

export const shareFileMacos = (sessionId, expectedContentGeneration) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("shareFileMacos", { jobId, sessionId, expectedContentGeneration }),
  ).promise;

/**
 * macOS Continuity Camera taraması; `{ jobId, promise, cancel }` döndürür. `promise`
 * `{ snapshot, kind }` ile çözülür; iptalde `SESSION_JOB_CANCELLED` ile reddedilir.
 */
export const startContinuityCameraScan = (
  sessionId,
  expectedContentGeneration,
  atIndex,
  pageFit,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("scanWithContinuityCamera", {
      jobId,
      sessionId,
      expectedContentGeneration,
      atIndex,
      pageFit,
    }),
  );

export const shareExternalFileMacos = (path) =>
  callBackend("shareExternalFileMacos", { path });

export const performOcrOnPage = (
  sessionId,
  expectedContentGeneration,
  pageIndex,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("performOcrOnPage", {
      jobId,
      sessionId,
      expectedContentGeneration,
      pageIndex,
    }),
  ).promise;
