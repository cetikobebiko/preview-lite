import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";

const generateRequestId = () => `req_${crypto.randomUUID()}`;
const generateAttemptId = () => `attempt_${crypto.randomUUID()}`;

export const configureAiEndpoint = (url, remoteApproved = false, privateNetworkApproved = false) =>
  callBackend("configureAiEndpoint", {
    url,
    remoteApproved,
    privateNetworkApproved,
  });

export const previewAiEndpoint = (url) =>
  callBackend("previewAiEndpoint", {
    url,
  });

export const getAiEndpointStatus = () => callBackend("getAiEndpointStatus");

export const clearAiEndpointConfig = () => callBackend("clearAiEndpointConfig");

export const cancelAiRequest = (requestId, sessionId = null, expectedContentGeneration = null, callerAttemptId = null) =>
  callBackend("cancelAiRequest", {
    requestId,
    ...(sessionId ? { sessionId } : {}),
    ...(expectedContentGeneration !== null ? { expectedContentGeneration } : {}),
    ...(callerAttemptId ? { callerAttemptId } : {}),
  });

export const buildSemanticIndex = (
  sessionId,
  expectedContentGeneration,
  chunkSize = 500,
  chunkOverlap = 50,
  embedModel = "nomic-embed-text",
  requestId = null,
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("buildSemanticIndex", {
      jobId,
      sessionId,
      expectedContentGeneration,
      chunkSize,
      chunkOverlap,
      embedModel,
      requestId: requestId || generateRequestId(),
    }),
  ).promise;

export const semanticSearch = (
  sessionId,
  expectedContentGeneration,
  query,
  topK = 3,
  embedModel = "nomic-embed-text",
  expectedChunkSize = 500,
  expectedChunkOverlap = 50,
  requestId = null,
) =>
  callBackend("semanticSearch", {
    sessionId,
    expectedContentGeneration,
    query,
    topK,
    embedModel,
    expectedChunkSize,
    expectedChunkOverlap,
    requestId: requestId || generateRequestId(),
  });

export const queryDocumentRag = (
  sessionId,
  expectedContentGeneration,
  prompt,
  history,
  llmModel = "llama3",
  embedModel = "nomic-embed-text",
  expectedChunkSize = 500,
  expectedChunkOverlap = 50,
  requestId = null,
) =>
  callBackend("queryDocumentRag", {
    sessionId,
    expectedContentGeneration,
    prompt,
    history,
    llmModel,
    embedModel,
    expectedChunkSize,
    expectedChunkOverlap,
    requestId: requestId || generateRequestId(),
  });

export const summarizeSelectionOrPage = (
  textContent,
  targetLength = "short",
  llmModel = "llama3",
  requestId = null,
  callerAttemptId = null,
) =>
  callBackend("summarizeSelectionOrPage", {
    textContent,
    targetLength,
    llmModel,
    requestId: requestId || generateRequestId(),
    callerAttemptId: callerAttemptId || generateAttemptId(),
  });
