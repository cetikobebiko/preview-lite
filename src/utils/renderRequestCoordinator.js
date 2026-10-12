import { createOpaqueId } from "../services/renderSession.js";

const EXPECTED_RENDER_ERRORS = new Set([
  "render_cancelled",
  "render_superseded",
  "render_inactive_document",
]);

function errorMessage(error) {
  return error?.message || String(error);
}

export function isExpectedRenderCancellation(error) {
  return EXPECTED_RENDER_ERRORS.has(errorMessage(error));
}

export function validateRenderResponse(response, request) {
  const width = Number(response?.width);
  const height = Number(response?.height);
  const data = response?.data;
  const expectedLength = width * height * 4;

  if (
    !Number.isInteger(width) ||
    !Number.isInteger(height) ||
    width <= 0 ||
    height <= 0 ||
    !Number.isSafeInteger(expectedLength) ||
    expectedLength <= 0 ||
    !(Array.isArray(data) || ArrayBuffer.isView(data)) ||
    data.length !== expectedLength
  ) {
    throw new Error("render_invalid_rgba_payload");
  }

  if (
    response.pageIndex !== request.pageIndex ||
    response.generation !== request.contentGeneration ||
    response.requestId !== request.requestId
  ) {
    throw new Error("render_invalid_response_metadata");
  }

  return response;
}

export function createRenderRequestCoordinator({
  scope,
  cancel,
  timeoutMs = 30_000,
  idFactory = () => createOpaqueId(scope),
} = {}) {
  let active = null;
  let destroyed = false;

  function cancelSnapshot(snapshot, reason) {
    try {
      Promise.resolve(cancel?.(snapshot, reason)).catch(() => {
        console.warn("Render cancellation IPC failed.");
      });
    } catch {
      console.warn("Render cancellation IPC failed.");
    }
  }

  // `reason` is diagnostic only: the backend logs it to its worker metrics.
  function invalidate(
    snapshot,
    { notifyBackend = true, reason = "unspecified" } = {},
  ) {
    if (!active || active.snapshot.requestId !== snapshot.requestId) return false;
    clearTimeout(active.timeoutId);
    active = null;
    if (notifyBackend) cancelSnapshot(snapshot, reason);
    return true;
  }

  function begin(baseSnapshot) {
    if (destroyed) throw new Error("render_component_destroyed");
    if (active) invalidate(active.snapshot, { reason: "superseded" });

    const snapshot = Object.freeze({
      ...baseSnapshot,
      scope,
      requestId: idFactory(),
    });

    let rejectTimeout;
    const timeoutPromise = new Promise((_, reject) => {
      rejectTimeout = reject;
    });
    const timeoutId = setTimeout(() => {
      if (!invalidate(snapshot, { reason: "timeout" })) return;
      rejectTimeout(new Error("render_timeout"));
    }, timeoutMs);
    active = { snapshot, timeoutId };

    return Object.freeze({
      snapshot,
      wait(renderPromise) {
        return Promise.race([
          Promise.resolve(renderPromise),
          timeoutPromise,
        ]);
      },
    });
  }

  function isCurrent(snapshot) {
    return (
      !destroyed &&
      active?.snapshot.requestId === snapshot.requestId
    );
  }

  function settle(snapshot) {
    if (!isCurrent(snapshot)) return false;
    clearTimeout(active.timeoutId);
    active = null;
    return true;
  }

  function destroy() {
    destroyed = true;
    if (active) invalidate(active.snapshot, { reason: "destroy" });
  }

  return Object.freeze({
    begin,
    cancelActive(reason = "unspecified") {
      return active ? invalidate(active.snapshot, { reason }) : false;
    },
    destroy,
    invalidate,
    isCurrent,
    settle,
  });
}
