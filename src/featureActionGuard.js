import { getFeaturePresentation } from "./featureRegistry.js";

export class FeatureUnavailableError extends Error {
  constructor(featureId, presentation) {
    const reason = presentation?.reason || presentation?.warning || "Feature is unavailable.";
    super(`${featureId || "unknown"}: ${reason}`);
    this.name = "FeatureUnavailableError";
    this.code = "FEATURE_UNAVAILABLE";
    this.featureId = featureId;
    this.presentation = presentation;
  }
}

export function assertFeatureRunnable(featureId, context = {}) {
  const presentation = getFeaturePresentation(featureId, context);
  if (!presentation.runnable) {
    throw new FeatureUnavailableError(featureId, presentation);
  }
  return presentation;
}

export async function guardFeatureAction(featureId, context, callback) {
  const presentation = assertFeatureRunnable(featureId, context);
  if (typeof callback !== "function") {
    throw new TypeError("Feature action callback must be a function.");
  }
  return await callback(presentation);
}
