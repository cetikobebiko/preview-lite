import {
  assertFeatureRunnable,
  FeatureUnavailableError,
  guardFeatureAction,
} from "../featureActionGuard.js";
import { getRuntimeFeatureContext } from "../featureRegistry.js";
import { startSensitiveDataScan } from "./backend/redaction.js";
import { startSignatureInspection } from "./backend/signatures.js";
import {
  performOcrOnPage,
  shareExternalFileMacos,
  shareFileMacos,
  speakText,
  startContinuityCameraScan,
  stopSpeaking,
} from "./backend/platform.js";

const runtimeContext = (context) => context ?? getRuntimeFeatureContext();

// Legacy `apply_redactions` stays fail-closed whatever the `redaction` feature
// says: secure redaction (plan/execute_secure_redaction) replaced it and the
// backend answers FEATURE_UNAVAILABLE before parsing. It never reaches the backend.
export const applyRedactionsAction = async () => {
  throw new FeatureUnavailableError("apply_redactions", {
    reason: "Eski apply_redactions komutu kapalıdır; güvenli redaksiyon akışını kullanın.",
  });
};

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

// Offline signature verification (Prompt 11 G2): trust is never evaluated (OPTION 1).
export const startSignatureInspectionAction = (
  sessionId,
  expectedContentGeneration,
  context,
) => {
  assertFeatureRunnable("signature_verification", runtimeContext(context));
  return startSignatureInspection(sessionId, expectedContentGeneration);
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
export const startContinuityCameraScanAction = (
  sessionId,
  expectedContentGeneration,
  atIndex,
  pageFit,
  context,
) => {
  assertFeatureRunnable("macos_continuity_camera", runtimeContext(context));
  return startContinuityCameraScan(
    sessionId,
    expectedContentGeneration,
    atIndex,
    pageFit,
  );
};

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
