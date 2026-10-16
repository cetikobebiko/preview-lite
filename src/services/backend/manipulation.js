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
