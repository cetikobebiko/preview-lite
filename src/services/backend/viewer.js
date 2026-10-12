import { callBackend, callBackendProfiled } from "./client.js";

export const renderPage = (request) =>
  callBackendProfiled("renderSessionPage", {
    sessionId: request.sessionId,
    pageIndex: request.pageIndex,
    scale: request.scale,
    rotation: request.rotation,
    contentGeneration: request.contentGeneration,
    requestId: request.requestId,
  });

export const renderThumbnail = (request) =>
  callBackendProfiled("renderSessionThumbnail", {
    sessionId: request.sessionId,
    pageIndex: request.pageIndex,
    targetWidth: request.targetWidth,
    contentGeneration: request.contentGeneration,
    requestId: request.requestId,
    scope: request.scope,
  });

export const setActiveRenderDocument = (request) =>
  callBackend("setActiveSessionRenderDocument", {
    sessionId: request.sessionId,
    contentGeneration: request.contentGeneration,
  });

export const cancelRenderRequest = (request, reason) =>
  callBackend("cancelSessionRenderRequest", {
    sessionId: request.sessionId,
    pageIndex: request.pageIndex,
    contentGeneration: request.contentGeneration,
    requestId: request.requestId,
    scope: request.scope,
    reason,
  });

export const getPageTextLayout = (
  sessionId,
  expectedContentGeneration,
  pageIndex,
) =>
  callBackend("getSessionPageTextLayout", {
    sessionId,
    expectedContentGeneration,
    pageIndex,
  });

// Lifts the backend quarantine of a page that repeatedly timed out; resolves
// to whether one was lifted. Only for an explicit user retry.
export const retryPage = (sessionId, expectedContentGeneration, pageIndex) =>
  callBackend("retrySessionPage", {
    sessionId,
    expectedContentGeneration,
    pageIndex,
  });

export const getPageSizes = (sessionId, expectedContentGeneration) =>
  callBackend("getSessionPageSizes", {
    sessionId,
    expectedContentGeneration,
  });
