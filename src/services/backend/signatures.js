import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";

export const listSystemCertificates = () =>
  callBackend("listSystemCertificates");

export const signPdfPades = (
  sessionId,
  expectedContentGeneration,
  outputPath,
  certId,
  pin,
  options,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("signPdfPades", {
      jobId,
      sessionId,
      expectedContentGeneration,
      outputPath,
      certId,
      pin,
      options,
    }),
  ).promise;

export const verifyPdfSignatures = (sessionId, expectedContentGeneration) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("verifyPdfSignatures", { jobId, sessionId, expectedContentGeneration }),
  ).promise;

export const enableLtv = (sessionId, expectedContentGeneration, signatureId) =>
  callBackend("enableLtv", { sessionId, expectedContentGeneration, signatureId });
