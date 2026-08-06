import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";

export const analyzeSessionColorSpaces = (
  sessionId,
  expectedContentGeneration,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("analyzeSessionColorSpaces", {
      jobId,
      sessionId,
      expectedContentGeneration,
    }),
  ).promise;

export const calculateSessionInkCoverage = (
  sessionId,
  expectedContentGeneration,
  pageIndex,
  maxLimit,
  scale,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("calculateSessionInkCoverage", {
      jobId,
      sessionId,
      expectedContentGeneration,
      pageIndex,
      maxLimit,
      scale,
    }),
  ).promise;

export const renderSessionSeparationChannels = (
  sessionId,
  expectedContentGeneration,
  pageIndex,
  scale,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("renderSessionSeparationChannels", {
      jobId,
      sessionId,
      expectedContentGeneration,
      pageIndex,
      scale,
    }),
  ).promise;

export const exportSessionPdfX = (
  sessionId,
  expectedContentGeneration,
  targetPath,
  standard,
  profileName,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("exportSessionPdfX", {
      jobId,
      sessionId,
      expectedContentGeneration,
      targetPath,
      standard,
      profileName,
    }),
  ).promise;
