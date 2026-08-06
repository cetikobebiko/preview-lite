import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { getSessionPdfOutline } from "../services/backend/session.js";
import Sidebar from "./Sidebar.svelte";

vi.mock("../services/backend/session.js", () => ({
  getSessionPdfOutline: vi.fn(),
}));
vi.mock("./Thumbnail.svelte", () => ({
  default: vi.fn(),
}));
vi.mock("./OutlineItem.svelte", () => ({
  default: vi.fn(),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Sidebar outline identity", () => {
  it("loads outline with sessionId and content generation", async () => {
    getSessionPdfOutline.mockResolvedValue([]);
    const view = render(Sidebar, {
      props: {
        sessionId: "outline-session",
        contentGeneration: 4,
        pageCount: 1,
      },
    });

    await fireEvent.click(view.getByRole("button", { name: "İçindekiler" }));
    await waitFor(() => {
      expect(getSessionPdfOutline).toHaveBeenCalledWith("outline-session", 4);
    });
  });

  it("ignores an outline response from the previous session", async () => {
    let resolveOld;
    getSessionPdfOutline
      .mockImplementationOnce(
        () => new Promise((resolve) => {
          resolveOld = resolve;
        }),
      )
      .mockResolvedValueOnce([]);

    const view = render(Sidebar, {
      props: {
        sessionId: "old-session",
        contentGeneration: 1,
      },
    });
    await fireEvent.click(view.getByRole("button", { name: "İçindekiler" }));
    await waitFor(() => expect(getSessionPdfOutline).toHaveBeenCalledTimes(1));

    await view.rerender({
      sessionId: "new-session",
      contentGeneration: 2,
    });
    resolveOld([{ title: "Old outline", page_index: 0, children: [] }]);
    await waitFor(() => expect(getSessionPdfOutline).toHaveBeenCalledTimes(2));
    expect(view.queryByText("Old outline")).toBeNull();
  });
});
