import { callBackend } from "./client.js";

/** Expands files and folders into PDF inputs (metadata only; nothing is parsed). */
export const scanBatchInputs = (paths, recursive = false) =>
  callBackend("scanBatchInputs", { paths, recursive });

/** Starts a batch; resolves to `{ batchId }`. Progress arrives as `batch_progress` events. */
export const startBatch = (request) => callBackend("startBatch", { request });

export const cancelBatch = (batchId) => callBackend("cancelBatch", { batchId });

/** Status of `batchId`, or of the current/last batch when `batchId` is null. */
export const getBatchStatus = (batchId = null) => callBackend("getBatchStatus", { batchId });

/** Opens the batch's recorded output folder; the backend takes no path from here. */
export const revealBatchOutputDir = (batchId) => callBackend("revealBatchOutputDir", { batchId });
