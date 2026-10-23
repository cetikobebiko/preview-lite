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
