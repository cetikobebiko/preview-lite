import { callBackend } from "./client.js";

const cleanupWarning = (cleanup) =>
  cleanup?.status === "retry_scheduled" || cleanup?.status === "deferred"
    ? cleanup.warning
    : null;

const consumeOutcome = (outcome) => {
  const warning = cleanupWarning(outcome?.cleanup);
  if (warning) {
    console.warn(`${warning.code}: ${warning.message}`);
  }

  if (outcome?.status === "completed") {
    return outcome.value;
  }

  const detail =
    outcome?.status === "failed"
      ? outcome.error
      : {
          code: "SESSION_JOB_CANCELLED",
          message: "Session job iptal edildi",
        };
  const error = new Error(detail?.message ?? "Session job tamamlanamadı");
  error.code = detail?.code ?? "SESSION_JOB_FAILED";
  error.jobOutcome = outcome;
  throw error;
};

export const runSessionJob = (sessionId, invokeOperation) => {
  const jobId = crypto.randomUUID();
  const outcomePromise = Promise.resolve().then(() => invokeOperation(jobId));
  return {
    jobId,
    promise: outcomePromise.then(consumeOutcome),
    cancel: () =>
      callBackend("cancelSessionJob", {
        sessionId,
        jobId,
      }),
  };
};
