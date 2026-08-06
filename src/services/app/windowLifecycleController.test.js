import { describe, expect, it, vi } from "vitest";
import { createWindowLifecycleController } from "./windowLifecycleController.js";

function createFixture(overrides = {}) {
  let closeRequested;
  const calls = [];
  const dependencies = {
    windowApi: {
      onCloseRequested: vi.fn(async (callback) => { closeRequested = callback; return vi.fn(); }),
      destroy: vi.fn(async () => calls.push("destroy")),
    },
    documentOpenCoordinator: {
      stop: vi.fn(async () => ({ status: "stopped" })),
      resume: vi.fn(() => calls.push("resume")),
      retryReconciliation: vi.fn(),
      retryPrecommitRecovery: vi.fn(),
    },
    getActiveSessionId: vi.fn(() => "A"),
    beginSessionTransition: vi.fn(async () => ({ token: "t", snapshot: { dirty: true } })),
    chooseTransition: vi.fn(async () => ({ action: "discard" })),
    saveTransition: vi.fn(async () => calls.push("save")),
    discardTransition: vi.fn(async () => calls.push("discard")),
    cancelTransition: vi.fn(async () => calls.push("cancel")),
    closeSession: vi.fn(() => calls.push("close-session")),
    closeAllDocumentSessions: vi.fn(async () => calls.push("close-all")),
    onError: vi.fn((error) => calls.push(["error", error.message])),
    ...overrides,
  };
  const controller = createWindowLifecycleController(dependencies);
  return { controller, dependencies, calls, getCloseRequested: () => closeRequested };
}

async function requestClose(fixture, count = 1) {
  await fixture.controller.start();
  const event = { preventDefault: vi.fn(() => fixture.calls.push("prevent")) };
  for (let index = 0; index < count; index += 1) fixture.getCloseRequested()(event);
  await fixture.controller.closeInFlight();
  return event;
}

describe("window lifecycle controller", () => {
  it("prevents first, runs one close transaction, and destroys after discard cleanup", async () => {
    const fixture = createFixture();
    const event = await requestClose(fixture, 2);
    expect(event.preventDefault).toHaveBeenCalledTimes(2);
    expect(fixture.calls).toEqual(["prevent", "prevent", "discard", "close-session", "close-all", "destroy"]);
    expect(fixture.dependencies.beginSessionTransition).toHaveBeenCalledOnce();
  });

  it("saves before closing the session and destroying the window", async () => {
    const fixture = createFixture({ chooseTransition: vi.fn(async () => ({ action: "save" })) });
    await requestClose(fixture);
    expect(fixture.calls).toEqual(["prevent", "save", "close-session", "close-all", "destroy"]);
  });

  it("cancels and resumes the coordinator without closing the session", async () => {
    const fixture = createFixture({ chooseTransition: vi.fn(async () => ({ action: "cancel" })) });
    await requestClose(fixture);
    expect(fixture.calls).toEqual(["prevent", "cancel", "resume"]);
    expect(fixture.dependencies.closeSession).not.toHaveBeenCalled();
    expect(fixture.dependencies.windowApi.destroy).not.toHaveBeenCalled();
  });

  it("reports transition failure and resumes without destroying", async () => {
    const fixture = createFixture({
      chooseTransition: vi.fn(async () => ({ action: "save" })),
      saveTransition: vi.fn(async () => { throw new Error("save failed"); }),
    });
    await requestClose(fixture);
    expect(fixture.calls).toEqual(["prevent", ["error", "save failed"], "resume"]);
    expect(fixture.dependencies.closeAllDocumentSessions).not.toHaveBeenCalled();
  });

  it("reports close-all failure, preserves the window, and resumes the coordinator", async () => {
    const closeFailure = new Error("close all failed");
    const fixture = createFixture({
      getActiveSessionId: () => null,
      closeAllDocumentSessions: vi.fn(async () => {
        fixture.calls.push("close-all");
        throw closeFailure;
      }),
    });

    await requestClose(fixture);

    expect(fixture.calls).toEqual([
      "prevent",
      "close-all",
      ["error", "close all failed"],
      "resume",
    ]);
    expect(fixture.dependencies.onError).toHaveBeenCalledWith(closeFailure);
    expect(fixture.dependencies.windowApi.destroy).not.toHaveBeenCalled();
    expect(fixture.dependencies.documentOpenCoordinator.resume).toHaveBeenCalledOnce();
  });

  it("reports native destroy failure and resumes because the window remains open", async () => {
    const destroyFailure = new Error("destroy failed");
    const fixture = createFixture({
      getActiveSessionId: () => null,
      windowApi: {
        onCloseRequested: vi.fn(async (callback) => {
          fixture.closeRequested = callback;
          return vi.fn();
        }),
        destroy: vi.fn(async () => {
          fixture.calls.push("destroy");
          throw destroyFailure;
        }),
      },
    });
    // Keep requestClose independent from the fixture's private registration closure.
    let closeRequested;
    fixture.dependencies.windowApi.onCloseRequested.mockImplementation(async (callback) => {
      closeRequested = callback;
      return vi.fn();
    });
    await fixture.controller.start();
    const event = { preventDefault: vi.fn(() => fixture.calls.push("prevent")) };
    closeRequested(event);
    await fixture.controller.closeInFlight();

    expect(fixture.calls).toEqual([
      "prevent",
      "close-all",
      "destroy",
      ["error", "destroy failed"],
      "resume",
    ]);
    expect(fixture.dependencies.onError).toHaveBeenCalledWith(destroyFailure);
    expect(fixture.dependencies.documentOpenCoordinator.resume).toHaveBeenCalledOnce();
  });

  it("keeps backend transition cleanup ordered before session and window cleanup", async () => {
    const fixture = createFixture({ chooseTransition: vi.fn(async () => ({ action: "save" })) });
    await requestClose(fixture);

    const saveOrder = fixture.dependencies.saveTransition.mock.invocationCallOrder[0];
    const closeAllOrder = fixture.dependencies.closeAllDocumentSessions.mock.invocationCallOrder[0];
    const destroyOrder = fixture.dependencies.windowApi.destroy.mock.invocationCallOrder[0];
    expect(saveOrder).toBeLessThan(closeAllOrder);
    expect(closeAllOrder).toBeLessThan(destroyOrder);
    expect(fixture.calls).toEqual(["prevent", "save", "close-session", "close-all", "destroy"]);
  });

  it("reconciles an unknown commit outcome before beginning close transition", async () => {
    const coordinator = {
      stop: vi.fn(async () => ({ status: "commit_outcome_unknown" })),
      retryReconciliation: vi.fn(async () => ({ status: "committed" })),
      retryPrecommitRecovery: vi.fn(),
      resume: vi.fn(),
    };
    const fixture = createFixture({ documentOpenCoordinator: coordinator, getActiveSessionId: () => null });
    await requestClose(fixture);
    expect(coordinator.retryReconciliation).toHaveBeenCalledOnce();
    expect(fixture.calls).toEqual(["prevent", "close-all", "destroy"]);
  });

  it("does not destroy when reconciliation remains unresolved", async () => {
    const coordinator = {
      stop: vi.fn(async () => ({ status: "commit_outcome_unknown" })),
      retryReconciliation: vi.fn(async () => ({ status: "commit_outcome_unknown" })),
      retryPrecommitRecovery: vi.fn(),
      resume: vi.fn(() => {}),
    };
    const fixture = createFixture({ documentOpenCoordinator: coordinator });
    await requestClose(fixture);
    expect(fixture.dependencies.onError).toHaveBeenCalledOnce();
    expect(coordinator.resume).toHaveBeenCalledOnce();
    expect(fixture.dependencies.windowApi.destroy).not.toHaveBeenCalled();
  });

  it("recovers required precommit cleanup before closing", async () => {
    const coordinator = {
      stop: vi.fn(async () => ({ status: "precommit_recovery_required" })),
      retryReconciliation: vi.fn(),
      retryPrecommitRecovery: vi.fn(async () => ({ status: "recovered" })),
      resume: vi.fn(),
    };
    const fixture = createFixture({ documentOpenCoordinator: coordinator, getActiveSessionId: () => null });
    await requestClose(fixture);
    expect(coordinator.retryPrecommitRecovery).toHaveBeenCalledOnce();
    expect(fixture.calls).toEqual(["prevent", "close-all", "destroy"]);
  });

  it("disposes a close listener that registers after destroy", async () => {
    let finishRegistration;
    const unlisten = vi.fn();
    const fixture = createFixture({
      windowApi: {
        onCloseRequested: vi.fn(() => new Promise((resolve) => { finishRegistration = () => resolve(unlisten); })),
        destroy: vi.fn(),
      },
    });
    const starting = fixture.controller.start();
    fixture.controller.destroy();
    finishRegistration();
    await starting;
    expect(unlisten).toHaveBeenCalledOnce();
  });
});
