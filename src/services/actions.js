import { assertFeatureRunnable, guardFeatureAction } from "../featureActionGuard.js";
import { getRuntimeFeatureContext } from "../featureRegistry.js";
import { mergePdfs } from "./backend/manipulation.js";
import {
  applyRedactions,
  startSensitiveDataScan,
} from "./backend/redaction.js";
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
