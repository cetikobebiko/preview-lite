import { describe, expect, it, vi } from "vitest";

vi.mock("../platform/dialogs.js", () => ({ ask: vi.fn(), open: vi.fn(), save: vi.fn() }));
vi.mock("../../configManager.js", () => ({
  addRecentFile: vi.fn(async () => undefined),
  getDocumentState: vi.fn(async () => null),
  saveDocumentState: vi.fn(async () => undefined),
}));
vi.mock("../backend/index.js", async (importOriginal) => ({
  ...(await importOriginal()),
  getSessionPageCount: vi.fn(async () => 3),
}));

const { createDocumentRuntime } = await import("./documentRuntime.js");

function setup(activeSessionId = "doc-a") {
  const documentSession = {
    activeSessionId,
    contentGeneration: 2,
    applySnapshot: vi.fn(() => true),
    get active() { return null; },
  };
  const viewer = { pageOrder: [0, 1, 2, 3], currentPageIndex: 3, historyStack: [1], forwardStack: [2], activeImageSourcePath: "" };
  const applyPageScopedMutation = vi.fn();
  const runtime = createDocumentRuntime({
    documentSession,
    renderSession: { snapshot: () => ({}) },
    viewer,
    annotation: {},
    viewerController: { activatePdf: vi.fn(async () => undefined) },
    ui: { closeModal: vi.fn() },
    tick: async () => undefined,
    getCanvas: () => null,
    getIsMounted: () => true,
    invalidateSearch: vi.fn(),
    onToast: vi.fn(),
    onError: vi.fn(),
    passwordPresentation: { show: vi.fn(), error: vi.fn(), dismiss: vi.fn() },
    applyPageScopedMutation,
  });
  return { runtime, viewer, documentSession, applyPageScopedMutation };
}

describe("documentRuntime.applyMutationSnapshot page remapping", () => {
  it("remaps page-scoped state for the active session before refreshing the page order", async () => {
    const { runtime, viewer, applyPageScopedMutation } = setup();
    const mapPageIndex = vi.fn();
    await runtime.applyMutationSnapshot({ id: "doc-a", contentGeneration: 2, stateRevision: 5 }, { mapPageIndex });
    expect(applyPageScopedMutation).toHaveBeenCalledWith("doc-a", { mapPageIndex });
    expect(viewer.pageOrder).toEqual([0, 1, 2]);
  });

  it("does not remap for mutations without a page mapping or for another session", async () => {
    const { runtime, applyPageScopedMutation } = setup();
    await runtime.applyMutationSnapshot({ id: "doc-a", contentGeneration: 2, stateRevision: 5 });
    await runtime.applyMutationSnapshot({ id: "doc-b", contentGeneration: 2, stateRevision: 5 }, { mapPageIndex: vi.fn() });
    expect(applyPageScopedMutation).not.toHaveBeenCalled();
  });

  it("still remaps when a newer snapshot already superseded this mutation's snapshot", async () => {
    const { runtime, documentSession, applyPageScopedMutation } = setup();
    documentSession.applySnapshot.mockReturnValue(false);
    await runtime.applyMutationSnapshot({ id: "doc-a", contentGeneration: 2, stateRevision: 5 }, { mapPageIndex: vi.fn() });
    expect(applyPageScopedMutation).toHaveBeenCalledTimes(1);
  });
});
