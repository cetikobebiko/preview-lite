import {
  normalizeSanitizationExecutionReport,
  normalizeSanitizationInspectionReport,
  SANITIZATION_POLICY_SCHEMA_VERSION,
} from "../../features/sanitization/sanitizationSchemas.js";
import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";

export function startSanitizationInspection(
  sessionId,
  expectedContentGeneration,
  policySchemaVersion = SANITIZATION_POLICY_SCHEMA_VERSION,
) {
  const handle = runSessionJob(sessionId, (jobId) =>
    callBackend("inspectDocumentSanitization", {
      jobId,
      sessionId,
      expectedContentGeneration,
      policySchemaVersion,
    }),
  );
  return Object.freeze({
    jobId: handle.jobId,
    promise: handle.promise.then(normalizeSanitizationInspectionReport),
    cancel: handle.cancel,
  });
}

export function startSanitizationExecution(
  sessionId,
  expectedContentGeneration,
  policy,
  targetPath,
) {
  const handle = runSessionJob(sessionId, (jobId) =>
    callBackend("executeDocumentSanitization", {
      jobId,
      sessionId,
      expectedContentGeneration,
      policy,
      targetPath,
    }),
  );
  return Object.freeze({
    jobId: handle.jobId,
    promise: handle.promise.then(normalizeSanitizationExecutionReport),
    cancel: handle.cancel,
  });
}
