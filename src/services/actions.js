import { assertFeatureRunnable, guardFeatureAction } from "../featureActionGuard.js";
import { getRuntimeFeatureContext } from "../featureRegistry.js";
import { mergePdfs } from "./backend/manipulation.js";
import {
  applyRedactions,
  sanitizePdfDocument,
  startSensitiveDataScan,
} from "./backend/redaction.js";
import {
  enableLtv,
  listSystemCertificates,
  signPdfPades,
  verifyPdfSignatures,
} from "./backend/signatures.js";
import { exportSessionPdfX } from "./backend/prepress.js";
import {
  performOcrOnPage,
  shareExternalFileMacos,
  shareFileMacos,
  speakText,
  stopSpeaking,
} from "./backend/platform.js";

const runtimeContext = (context) => context ?? getRuntimeFeatureContext();

export const mergePdfsDirectAction = (
  filePaths,
  outputPath,
  context,
) =>
  guardFeatureAction(
    "page_merge_direct",
    runtimeContext(context),
    () => mergePdfs(filePaths, outputPath),
  );

export const applyRedactionsAction = (
  sessionId,
  expectedContentGeneration,
  outputPath,
  redactions,
  context,
) =>
  guardFeatureAction(
    "redaction",
    runtimeContext(context),
    () => applyRedactions(
      sessionId,
      expectedContentGeneration,
      outputPath,
      redactions,
    ),
  );

export const sanitizePdfDocumentAction = (
  sessionId,
  expectedContentGeneration,
  outputPath,
  options,
  context,
) =>
  guardFeatureAction(
    "sanitization",
    runtimeContext(context),
    () => sanitizePdfDocument(
      sessionId,
      expectedContentGeneration,
      outputPath,
      options,
    ),
  );

export const startSensitiveDataScanAction = (
  sessionId,
  expectedContentGeneration,
  patterns,
  context,
) => {
  assertFeatureRunnable("sensitive_data_scan", runtimeContext(context));
  return startSensitiveDataScan(
    sessionId,
    expectedContentGeneration,
    patterns,
  );
};

export const scanSensitiveDataAction = (
  sessionId,
  expectedContentGeneration,
  patterns,
  context,
) => {
  try {
    return startSensitiveDataScanAction(
      sessionId,
      expectedContentGeneration,
      patterns,
      context,
    ).promise;
  } catch (error) {
    return Promise.reject(error);
  }
};

export const listSystemCertificatesAction = (context) =>
  guardFeatureAction(
    "pades_signing",
    runtimeContext(context),
    () => listSystemCertificates(),
  );

export const signPdfPadesAction = (
  sessionId,
  expectedContentGeneration,
  outputPath,
  certId,
  pin,
  options,
  context,
) =>
  guardFeatureAction(
    "pades_signing",
    runtimeContext(context),
    () => signPdfPades(
      sessionId,
      expectedContentGeneration,
      outputPath,
      certId,
      pin,
      options,
    ),
  );

export const verifyPdfSignaturesAction = (
  sessionId,
  expectedContentGeneration,
  context,
) =>
  guardFeatureAction(
    "signature_verification",
    runtimeContext(context),
    () => verifyPdfSignatures(sessionId, expectedContentGeneration),
  );

export const enableLtvAction = (
  sessionId,
  expectedContentGeneration,
  signatureId,
  context,
) =>
  guardFeatureAction(
    "ltv",
    runtimeContext(context),
    () => enableLtv(sessionId, expectedContentGeneration, signatureId),
  );

export const exportPdfXAction = (
  sessionId,
  expectedContentGeneration,
  targetPath,
  standard,
  profileName,
  context,
) =>
  guardFeatureAction(
    "pdf_x_export",
    runtimeContext(context),
    () => exportSessionPdfX(
      sessionId,
      expectedContentGeneration,
      targetPath,
      standard,
      profileName,
    ),
  );

export const speakTextAction = (text, voice, context) =>
  guardFeatureAction(
    "text_to_speech",
    runtimeContext(context),
    () => speakText(text, voice),
  );

export const stopSpeakingAction = (context) =>
  guardFeatureAction(
    "text_to_speech",
    runtimeContext(context),
    () => stopSpeaking(),
  );

export const shareFileMacosAction = (
  sessionId,
  expectedContentGeneration,
  context,
) =>
  guardFeatureAction(
    "macos_share",
    runtimeContext(context),
    () => shareFileMacos(sessionId, expectedContentGeneration),
  );

export const shareExternalFileMacosAction = (path, context) =>
  guardFeatureAction(
    "macos_share",
    runtimeContext(context),
    () => shareExternalFileMacos(path),
  );

export const performOcrOnPageAction = (
  sessionId,
  expectedContentGeneration,
  pageIndex,
  context,
) =>
  guardFeatureAction(
    "macos_vision_ocr",
    runtimeContext(context),
    () => performOcrOnPage(
      sessionId,
      expectedContentGeneration,
      pageIndex,
    ),
  );
