import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";

export const mergePdfs = (filePaths, outputPath) =>
  callBackend("mergePdfs", { filePaths, outputPath });

export const mergeSessionPdfs = (sessionId, expectedContentGeneration, additionalPaths, targetPath) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("mergeSessionPdfs", { jobId, sessionId, expectedContentGeneration, additionalPaths, targetPath }),
  ).promise;

export const splitSessionPdf = (sessionId, expectedContentGeneration, ranges, outputDir, overwrite = false) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("splitSessionPdf", { jobId, sessionId, expectedContentGeneration, ranges, outputDir, overwrite }),
  ).promise;

// Returns { jobId, promise, cancel }; the promise resolves to the comparison.
export const startDocumentCompare = (sessionId, expectedContentGeneration, otherPath, currentIsOld) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("compareSessionPdf", { jobId, sessionId, expectedContentGeneration, otherPath, currentIsOld }),
  );
