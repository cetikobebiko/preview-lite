import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";
import {
  validatePrintResourceInspectionReport,
  validateRgbDerivedCoveragePreview,
  validateRgbDerivedProcessPreview,
} from "./prepressSchemas.js";

const validateHandle = (handle, validator, expected) => ({
  ...handle,
  promise: handle.promise.then((value) => validator(value, expected)),
});

export const inspectSessionPrintResources = (
  sessionId,
  expectedContentGeneration,
  physicalPageIndex,
  requestId,
) => {
  const expected = { sessionId, contentGeneration: expectedContentGeneration, physicalPageIndex, requestId };
  return validateHandle(runSessionJob(sessionId, (jobId) =>
    callBackend("inspectSessionPrintResources", {
    jobId,
    sessionId,
    expectedContentGeneration,
    physicalPageIndex,
      requestId,
    })), validatePrintResourceInspectionReport, expected);
};

export const generateSessionRgbDerivedCoveragePreview = (
  sessionId,
  expectedContentGeneration,
  physicalPageIndex,
  requestId,
  thresholdPercent,
  scale,
) => {
  const expected = { sessionId, contentGeneration: expectedContentGeneration, physicalPageIndex, requestId };
  return validateHandle(runSessionJob(sessionId, (jobId) =>
    callBackend("generateSessionRgbDerivedCoveragePreview", {
    jobId,
    sessionId,
    expectedContentGeneration,
    physicalPageIndex,
    requestId,
    thresholdPercent,
      scale,
    })), validateRgbDerivedCoveragePreview, expected);
};

export const generateSessionRgbDerivedProcessPreview = (
  sessionId,
  expectedContentGeneration,
  physicalPageIndex,
  requestId,
  scale,
) => {
  const expected = { sessionId, contentGeneration: expectedContentGeneration, physicalPageIndex, requestId };
  return validateHandle(runSessionJob(sessionId, (jobId) =>
    callBackend("generateSessionRgbDerivedProcessPreview", {
    jobId,
    sessionId,
    expectedContentGeneration,
    physicalPageIndex,
    requestId,
      scale,
    })), validateRgbDerivedProcessPreview, expected);
};
