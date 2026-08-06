import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";

export const readImageFile = (path, rotation, flipH, flipV) =>
  callBackend("readImageFile", { path, rotation, flipH, flipV });

export const convertAndSaveImage = (
  sourcePath,
  targetPath,
  format,
  rotation,
  flipH,
  flipV,
) =>
  callBackend("convertAndSaveImage", {
    sourcePath,
    targetPath,
    format,
    rotation,
    flipH,
    flipV,
  });

export const burnAnnotationsToPdf = (
  sessionId,
  expectedContentGeneration,
  annotationsJson,
  targetPath,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("burnAnnotationsToPdf", {
      jobId,
      sessionId,
      expectedContentGeneration,
      annotationsJson,
      targetPath,
    }),
  ).promise;
