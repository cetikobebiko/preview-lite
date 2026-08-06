import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";

export const buildSemanticIndex = (
  sessionId,
  expectedContentGeneration,
  chunkSize = 500,
  chunkOverlap = 50,
  ollamaUrl = "http://127.0.0.1:11434",
  embedModel = "nomic-embed-text",
) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("buildSemanticIndex", {
      jobId,
      sessionId,
      expectedContentGeneration,
      chunkSize,
      chunkOverlap,
      ollamaUrl,
      embedModel,
    }),
  ).promise;

export const semanticSearch = (
  sessionId,
  expectedContentGeneration,
  query,
  topK = 3,
  ollamaUrl = "http://127.0.0.1:11434",
  embedModel = "nomic-embed-text",
) =>
  callBackend("semanticSearch", {
    sessionId,
    expectedContentGeneration,
    query,
    topK,
    ollamaUrl,
    embedModel,
  });

export const queryDocumentRag = (
  sessionId,
  expectedContentGeneration,
  prompt,
  history,
  ollamaUrl = "http://127.0.0.1:11434",
  llmModel = "llama3",
  embedModel = "nomic-embed-text",
) =>
  callBackend("queryDocumentRag", {
    sessionId,
    expectedContentGeneration,
    prompt,
    history,
    ollamaUrl,
    llmModel,
    embedModel,
  });

export const summarizeSelectionOrPage = (
  textContent,
  targetLength = "short",
  ollamaUrl = "http://127.0.0.1:11434",
  llmModel = "llama3",
) =>
  callBackend("summarizeSelectionOrPage", {
    textContent,
    targetLength,
    ollamaUrl,
    llmModel,
  });
