import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/svelte";
import {
  getSessionStructureTree,
  validateSessionPdfUa,
} from "../services/backend/session.js";
import A11yPanel from "./A11yPanel.svelte";

vi.mock("../services/backend/session.js", () => ({
  getSessionStructureTree: vi.fn(),
  validateSessionPdfUa: vi.fn(),
}));
vi.mock("./FeatureNotice.svelte", () => ({
  default: vi.fn(),
}));
vi.mock("./TagTreeExplorer.svelte", () => ({
  default: vi.fn(),
}));
vi.mock("../featureRegistry.js", () => ({
  getRuntimeFeatureContext: vi.fn(() => ({})),
  getFeaturePresentation: vi.fn(() => ({ runnable: false })),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("A11yPanel session reads", () => {
  it("performs one validation/tree fetch pair for a session generation", async () => {
    validateSessionPdfUa.mockResolvedValue({
      score: 90,
      issues: [],
      is_tagged: true,
    });
    getSessionStructureTree.mockResolvedValue({
      tag_type: "DocumentRoot",
      children: [],
    });

    const view = render(A11yPanel, {
      props: {
        sessionId: "a11y-session",
        contentGeneration: 9,
      },
    });

    await waitFor(() => expect(view.getByText("90/100")).toBeTruthy());
    expect(validateSessionPdfUa).toHaveBeenCalledTimes(1);
    expect(validateSessionPdfUa).toHaveBeenCalledWith("a11y-session", 9);
    expect(getSessionStructureTree).toHaveBeenCalledTimes(1);
    expect(getSessionStructureTree).toHaveBeenCalledWith("a11y-session", 9);
  });

  it("does not show validation data returned for an old session", async () => {
    let resolveOld;
    validateSessionPdfUa
      .mockImplementationOnce(
        () => new Promise((resolve) => {
          resolveOld = resolve;
        }),
      )
      .mockResolvedValueOnce({ score: 88, issues: [], is_tagged: true });
    getSessionStructureTree.mockResolvedValue({
      tag_type: "DocumentRoot",
      children: [],
    });

    const view = render(A11yPanel, {
      props: { sessionId: "old-session", contentGeneration: 1 },
    });
    await waitFor(() => expect(validateSessionPdfUa).toHaveBeenCalledTimes(1));

    await view.rerender({
      sessionId: "new-session",
      contentGeneration: 2,
    });
    await waitFor(() => expect(view.getByText("88/100")).toBeTruthy());

    resolveOld({ score: 12, issues: ["stale"], is_tagged: false });
    await Promise.resolve();
    expect(view.queryByText("12/100")).toBeNull();
    expect(view.queryByText("stale")).toBeNull();
  });
});
