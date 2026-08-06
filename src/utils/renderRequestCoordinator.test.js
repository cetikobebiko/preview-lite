import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createRenderRequestCoordinator,
  isExpectedRenderCancellation,
  validateRenderResponse,
} from "./renderRequestCoordinator.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

function baseRequest(pageIndex = 0) {
  return {
    sessionId: "document-1",
    pageIndex,
    contentGeneration: 3,
  };
}

function validResponse(request, width = 2, height = 2) {
  return {
    width,
    height,
    data: new Uint8Array(width * height * 4),
    pageIndex: request.pageIndex,
    generation: request.contentGeneration,
    requestId: request.requestId,
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("render request coordinator", () => {
  it("supersedes only the previous request in its own page/scope coordinator", () => {
    const cancel = vi.fn();
    const ids = ["old", "new"];
    const coordinator = createRenderRequestCoordinator({
      scope: "main_page",
      cancel,
      idFactory: () => ids.shift(),
    });

    const oldRequest = coordinator.begin(baseRequest(0)).snapshot;
    const newRequest = coordinator.begin(baseRequest(0)).snapshot;

    expect(cancel).toHaveBeenCalledOnce();
    expect(cancel).toHaveBeenCalledWith(oldRequest);
    expect(coordinator.isCurrent(oldRequest)).toBe(false);
    expect(coordinator.isCurrent(newRequest)).toBe(true);
  });

  it("keeps different pages and thumbnail consumers isolated", () => {
    const cancelMain = vi.fn();
    const cancelSidebar = vi.fn();
    const main = createRenderRequestCoordinator({
      scope: "main_page",
      cancel: cancelMain,
      idFactory: () => "main",
    });
    const sidebar = createRenderRequestCoordinator({
      scope: "sidebar_thumbnail",
      cancel: cancelSidebar,
      idFactory: () => "sidebar",
    });

    const mainRequest = main.begin(baseRequest(0)).snapshot;
    const sidebarRequest = sidebar.begin(baseRequest(0)).snapshot;

    expect(main.isCurrent(mainRequest)).toBe(true);
    expect(sidebar.isCurrent(sidebarRequest)).toBe(true);
    expect(cancelMain).not.toHaveBeenCalled();
    expect(cancelSidebar).not.toHaveBeenCalled();
  });

  it("invalidates locally, cancels backend and continues observing the invoke after timeout", async () => {
    vi.useFakeTimers();
    const cancel = vi.fn().mockRejectedValue(new Error("cancel IPC failed"));
    const render = deferred();
    const coordinator = createRenderRequestCoordinator({
      scope: "main_page",
      cancel,
      timeoutMs: 50,
      idFactory: () => "timed",
    });
    const request = coordinator.begin(baseRequest());
    const waiting = request.wait(render.promise);
    const timeoutAssertion = expect(waiting).rejects.toThrow("render_timeout");

    await vi.advanceTimersByTimeAsync(50);

    await timeoutAssertion;
    expect(cancel).toHaveBeenCalledWith(request.snapshot);
    expect(coordinator.isCurrent(request.snapshot)).toBe(false);
    render.reject(new Error("late backend failure"));
    await Promise.resolve();
  });

  it("cancels the exact active request on component destroy", () => {
    const cancel = vi.fn();
    const coordinator = createRenderRequestCoordinator({
      scope: "organizer_thumbnail",
      cancel,
      idFactory: () => "organizer",
    });
    const request = coordinator.begin(baseRequest(4)).snapshot;

    coordinator.destroy();

    expect(cancel).toHaveBeenCalledWith(request);
    expect(coordinator.isCurrent(request)).toBe(false);
    expect(() => coordinator.begin(baseRequest(4))).toThrow(
      "render_component_destroyed",
    );
  });

  it("rejects stale metadata and malformed or missing RGBA payloads", () => {
    const request = {
      ...baseRequest(),
      scope: "main_page",
      requestId: "current",
    };
    expect(validateRenderResponse(validResponse(request), request)).toMatchObject({
      requestId: "current",
    });
    expect(() =>
      validateRenderResponse(
        { ...validResponse(request), requestId: "stale" },
        request,
      ),
    ).toThrow("render_invalid_response_metadata");
    expect(() =>
      validateRenderResponse(
        { ...validResponse(request), data: new Uint8Array(3) },
        request,
      ),
    ).toThrow("render_invalid_rgba_payload");
    expect(() =>
      validateRenderResponse(
        { ...validResponse(request), data: undefined },
        request,
      ),
    ).toThrow("render_invalid_rgba_payload");
  });

  it("classifies cancellation separately from real render failures", () => {
    expect(isExpectedRenderCancellation(new Error("render_cancelled"))).toBe(true);
    expect(isExpectedRenderCancellation(new Error("render_superseded"))).toBe(true);
    expect(isExpectedRenderCancellation(new Error("pdfium_failed"))).toBe(false);
  });
});
