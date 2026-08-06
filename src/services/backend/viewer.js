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

export const cancelRenderRequest = (request) =>
  callBackend("cancelSessionRenderRequest", {
    sessionId: request.sessionId,
    pageIndex: request.pageIndex,
    contentGeneration: request.contentGeneration,
    requestId: request.requestId,
    scope: request.scope,
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
