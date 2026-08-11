import { normalizeSanitizationInspectionReport, SANITIZATION_POLICY_SCHEMA_VERSION } from "../../features/sanitization/sanitizationSchemas.js";
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
