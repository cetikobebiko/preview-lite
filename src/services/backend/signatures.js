import { normalizeSignatureReport } from "../../features/signature/signatureReport.js";
import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";

// Read-only offline signature verification of the session snapshot (Prompt 11 G2). The
// job worker parses the PDF and CMS; only a bounded, typed report comes back.
export function startSignatureInspection(sessionId, expectedContentGeneration) {
  const handle = runSessionJob(sessionId, (jobId) =>
    callBackend("inspectSessionSignatures", {
      jobId,
      sessionId,
      expectedContentGeneration,
    }),
  );
  return Object.freeze({
    jobId: handle.jobId,
    promise: handle.promise.then(normalizeSignatureReport),
    cancel: handle.cancel,
  });
}
