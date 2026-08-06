import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/svelte";
import { getPageAnnotations } from "../pdfState.js";
import AnnotationLayer from "./AnnotationLayer.svelte";

vi.mock("../pdfState.js", () => ({
  getPageAnnotations: vi.fn(),
  addAnnotation: vi.fn(),
  deleteAnnotation: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn(),
}));
vi.mock("../services/asyncPrompt.svelte.js", () => ({
  requireAsyncPromptCoordinator: vi.fn(() => ({
    request: vi.fn(),
  })),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("AnnotationLayer session identity", () => {
  it("loads annotations with sessionId and content generation", async () => {
    getPageAnnotations.mockResolvedValue([]);
    render(AnnotationLayer, {
      props: {
        sessionId: "annotation-session",
        contentGeneration: 5,
        pageIndex: 2,
      },
    });

    await waitFor(() => {
      expect(getPageAnnotations).toHaveBeenCalledWith(
        "annotation-session",
        5,
        2,
      );
    });
  });

  it("does not paint annotations returned for an old session", async () => {
    let resolveOld;
    getPageAnnotations
      .mockImplementationOnce(
        () => new Promise((resolve) => {
          resolveOld = resolve;
        }),
      )
      .mockResolvedValueOnce([]);

    const view = render(AnnotationLayer, {
      props: {
        sessionId: "old-session",
        contentGeneration: 1,
        pageIndex: 0,
        pageWidth: 100,
        pageHeight: 100,
      },
    });
    await waitFor(() => expect(getPageAnnotations).toHaveBeenCalledTimes(1));

    await view.rerender({
      sessionId: "new-session",
      contentGeneration: 2,
    });
    await waitFor(() => expect(getPageAnnotations).toHaveBeenCalledTimes(2));

    resolveOld([
      {
        id: "old-annotation",
        annot_type: "Highlight",
        rect: { x: 0, y: 0, width: 10, height: 10 },
        color: "#ffff00",
      },
    ]);
    await Promise.resolve();
    expect(view.queryByTitle("Highlight")).toBeNull();
  });
});
