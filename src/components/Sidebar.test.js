import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { getSessionPdfOutline } from "../services/backend/session.js";
import Sidebar from "./Sidebar.svelte";
import Thumbnail from "./Thumbnail.svelte";
import OutlineItem from "./OutlineItem.svelte";

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

describe("Sidebar page order", () => {
  it("lists thumbnails in display order and marks the current display page active", () => {
    const onJump = vi.fn();
    render(Sidebar, {
      props: { sessionId: "s", contentGeneration: 1, pageOrder: [2, 0, 1], currentPageIndex: 1, onJump },
    });
    const props = Thumbnail.mock.calls.map((call) => call[1]);
    expect(props.map((entry) => [entry.pageIndex, entry.pageNumber, entry.isActive])).toEqual([
      [2, 1, false],
      [0, 2, true],
      [1, 3, false],
    ]);
    props[2].onSelect();
    expect(onJump).toHaveBeenCalledWith(2);
  });

  it("routes outline page_index (physical) through the physical jump", async () => {
    getSessionPdfOutline.mockResolvedValue([{ title: "Bölüm", page_index: 2, children: [] }]);
    const onJump = vi.fn();
    const onJumpPhysical = vi.fn();
    const view = render(Sidebar, {
      props: { sessionId: "s", contentGeneration: 1, pageOrder: [2, 0, 1], onJump, onJumpPhysical },
    });
    await fireEvent.click(view.getByRole("button", { name: "İçindekiler" }));
    await waitFor(() => expect(OutlineItem).toHaveBeenCalled());
    const outlineProps = OutlineItem.mock.calls.at(-1)[1];
    outlineProps.onJump(outlineProps.node.page_index);
    expect(onJumpPhysical).toHaveBeenCalledWith(2);
    expect(onJump).not.toHaveBeenCalled();
  });
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
