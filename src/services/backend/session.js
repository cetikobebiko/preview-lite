import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";
import {
  normalizeAccessibilityAuditReport,
  normalizeAccessibilityStructureInspection,
} from "../../features/accessibility/accessibilitySchemas.js";

const sessionSaveInProgress = new Map();

export const openDocumentSession = (path) =>
  callBackend("openDocumentSession", { path });

export const authenticateDocumentSession = (sessionId, password) =>
  callBackend("authenticateDocumentSession", { sessionId, password });

export const getDocumentSessionSnapshot = (sessionId) =>
  callBackend("getDocumentSessionSnapshot", { sessionId });

export const getActiveDocumentSessionSnapshot = () =>
  callBackend("getActiveDocumentSessionSnapshot");

export const probeCandidateRender = (sessionId, expectedContentGeneration) =>
  callBackend("probeCandidateRender", { sessionId, expectedContentGeneration });

export const recoverDocumentSession = (sessionId) =>
  callBackend("recoverDocumentSession", { sessionId });

export const cancelSessionJob = (sessionId, jobId) =>
  callBackend("cancelSessionJob", { sessionId, jobId });

export const exportSessionPageToImage = (
  sessionId,
  expectedContentGeneration,
  pageIndex,
  targetPath,
  format,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("exportSessionPageToImage", {
      jobId,
      sessionId,
      expectedContentGeneration,
      pageIndex,
      targetPath,
      format,
    }),
  ).promise;

export const exportSessionPagesAsImages = (
  sessionId,
  expectedContentGeneration,
  pages,
  outputDir,
  format,
  dpi,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("exportSessionPagesAsImages", {
      jobId,
      sessionId,
      expectedContentGeneration,
      pages,
      outputDir,
      format,
      dpi,
    }),
  ).promise;

export const exportSessionTextContent = (
  sessionId,
  expectedContentGeneration,
  pages,
  format,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("exportSessionTextContent", {
      jobId,
      sessionId,
      expectedContentGeneration,
      pages,
      format,
    }),
  ).promise;

export const printSessionPdf = (
  sessionId,
  expectedContentGeneration,
  pageRange = null,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("printSessionPdf", {
      jobId,
      sessionId,
      expectedContentGeneration,
      pageRange,
    }),
  ).promise;

export const extractSessionPages = (
  sessionId,
  expectedContentGeneration,
  pageIndices,
  targetPath,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("extractSessionPages", {
      jobId,
      sessionId,
      expectedContentGeneration,
      pageIndices,
      targetPath,
    }),
  ).promise;

export const discardCandidateSession = (sessionId) =>
  callBackend("discardCandidateSession", { sessionId });

export const getSessionPageCount = (sessionId, expectedContentGeneration) =>
  callBackend("getSessionPageCount", { sessionId, expectedContentGeneration });

export const getSessionPdfOutline = (sessionId, expectedContentGeneration) =>
  callBackend("getSessionPdfOutline", { sessionId, expectedContentGeneration });

export const getSessionPdfMetadata = (sessionId, expectedContentGeneration) =>
  callBackend("getSessionPdfMetadata", { sessionId, expectedContentGeneration });

export const getSessionStructureTree = (sessionId, expectedContentGeneration) =>
  callBackend("getSessionStructureTree", {
    sessionId,
    expectedContentGeneration,
  }).then(normalizeAccessibilityStructureInspection);

export const auditSessionAccessibility = (sessionId, expectedContentGeneration) =>
  callBackend("auditSessionAccessibility", {
    sessionId,
    expectedContentGeneration,
  }).then(normalizeAccessibilityAuditReport);

export const searchSessionPdfText = (
  sessionId,
  expectedContentGeneration,
  query,
) =>
  callBackend("searchSessionPdfText", {
    sessionId,
    expectedContentGeneration,
    query,
  });

export const getSessionPageTextLayout = (
  sessionId,
  expectedContentGeneration,
  pageIndex,
) =>
  callBackend("getSessionPageTextLayout", {
    sessionId,
    expectedContentGeneration,
    pageIndex,
  });

export const renderSessionPage = (request) =>
  callBackend("renderSessionPage", request);

export const renderSessionThumbnail = (request) =>
  callBackend("renderSessionThumbnail", request);

export const setActiveSessionRenderDocument = (
  sessionId,
  contentGeneration,
) =>
  callBackend("setActiveSessionRenderDocument", {
    sessionId,
    contentGeneration,
  });

export const cancelSessionRenderRequest = (request) =>
  callBackend("cancelSessionRenderRequest", request);

export const rotateSessionPage = (
  sessionId,
  expectedContentGeneration,
  pageIndex,
  degrees,
) =>
  callBackend("rotateSessionPage", {
    sessionId,
    expectedContentGeneration,
    pageIndex,
    degrees,
  });

export const deleteSessionPages = (
  sessionId,
  expectedContentGeneration,
  pageIndices,
) =>
  callBackend("deleteSessionPages", {
    sessionId,
    expectedContentGeneration,
    pageIndices,
  });

export const reorderSessionPages = (
  sessionId,
  expectedContentGeneration,
  newOrder,
) =>
  callBackend("reorderSessionPages", {
    sessionId,
    expectedContentGeneration,
    newOrder,
  });

export const insertSessionBlankPage = (
  sessionId,
  expectedContentGeneration,
  atIndex,
  width,
  height,
) =>
  callBackend("insertSessionBlankPage", {
    sessionId,
    expectedContentGeneration,
    atIndex,
    width,
    height,
  });

export const getSessionPageAnnotations = (
  sessionId,
  expectedContentGeneration,
  pageIndex,
) =>
  callBackend("getSessionPageAnnotations", {
    sessionId,
    expectedContentGeneration,
    pageIndex,
  });

export const addSessionAnnotation = (
  sessionId,
  expectedContentGeneration,
  pageIndex,
  annotType,
  normalizedRect,
  color,
  contents,
) =>
  callBackend("addSessionAnnotation", {
    sessionId,
    expectedContentGeneration,
    pageIndex,
    annotType,
    normalizedRect,
    color,
    contents,
  });

export const deleteSessionAnnotation = (
  sessionId,
  expectedContentGeneration,
  pageIndex,
  annotId,
) =>
  callBackend("deleteSessionAnnotation", {
    sessionId,
    expectedContentGeneration,
    pageIndex,
    annotId,
  });

export const updateSessionMetadata = (
  sessionId,
  expectedContentGeneration,
  metadata,
) =>
  callBackend("updateSessionMetadata", {
    sessionId,
    expectedContentGeneration,
    metadata,
  });

export const saveDocumentSession = async (sessionId, expectedStateRevision) => {
  if (sessionSaveInProgress.get(sessionId)) {
    throw new Error("Save operation is already in progress for this session.");
  }
  sessionSaveInProgress.set(sessionId, true);
  try {
    return await callBackend("saveDocumentSession", { sessionId, expectedStateRevision });
  } finally {
    sessionSaveInProgress.set(sessionId, false);
  }
};

export const saveDocumentSessionAs = async (sessionId, targetPath, expectedStateRevision) => {
  if (sessionSaveInProgress.get(sessionId)) {
    throw new Error("Save operation is already in progress for this session.");
  }
  sessionSaveInProgress.set(sessionId, true);
  try {
    return await callBackend("saveDocumentSessionAs", { sessionId, targetPath, expectedStateRevision });
  } finally {
    sessionSaveInProgress.set(sessionId, false);
  }
};

export const beginSessionTransition = (sessionId, kind) =>
  callBackend("beginSessionTransition", { sessionId, kind });

export const saveTransition = (token, targetPath = null) =>
  callBackend("saveTransition", { token, targetPath });

export const discardTransition = (token) =>
  callBackend("discardTransition", { token });

export const cancelTransition = (token) =>
  callBackend("cancelTransition", { token });

export const prepareCandidateActivation = (
  candidateSessionId,
  previousSessionId,
  previousTransitionToken = null,
) =>
  callBackend("prepareCandidateActivation", {
    candidateSessionId,
    previousSessionId,
    previousTransitionToken,
  });

export const commitCandidateActivation = (activationToken) =>
  callBackend("commitCandidateActivation", { activationToken });

export const rollbackCandidateActivation = (activationToken) =>
  callBackend("rollbackCandidateActivation", { activationToken });

export const closeAllDocumentSessions = () =>
  callBackend("closeAllDocumentSessions");
