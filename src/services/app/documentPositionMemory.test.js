import { describe, expect, it, vi } from "vitest";
import { createDocumentPositionMemory } from "./documentPositionMemory.js";

function setup(overrides = {}) {
  const viewer = {
    currentPageIndex: 1,
    pageOrder: [0, 1, 2],
    scale: 1,
    isBookMode: false,
    isSinglePageMode: false,
  };
  const saveState = vi.fn(async () => undefined);
  const getState = vi.fn(() => null);
  const scrollToPage = vi.fn();
  const schedule = vi.fn((callback) => callback());
  const positionMemory = createDocumentPositionMemory({
    viewer,
    getPath: () => "/tmp/document.pdf",
    getState,
    saveState,
    scrollToPage,
    schedule,
    ...overrides,
  });
  return { viewer, saveState, getState, scrollToPage, schedule, positionMemory };
}

describe("documentPositionMemory", () => {
  it("persists a viewer snapshot without retaining a second state copy", async () => {
    const { positionMemory, saveState } = setup();
    positionMemory.save();
    await Promise.resolve();

    expect(saveState).toHaveBeenCalledWith("/tmp/document.pdf", {
      pageIndex: 1,
      scale: 1,
      isBookMode: false,
      isSinglePageMode: false,
    });
  });

  it("clamps a restored state and schedules its page scroll", () => {
    const getState = vi.fn(() => ({ pageIndex: 99, scale: 9, isBookMode: true, isSinglePageMode: true }));
    const { viewer, positionMemory, scrollToPage, schedule } = setup({ getState });

    expect(positionMemory.restore("/tmp/document.pdf")).toBe(true);
    expect(viewer).toMatchObject({ currentPageIndex: 2, scale: 4, isBookMode: true, isSinglePageMode: true });
    expect(schedule).toHaveBeenCalledWith(expect.any(Function), 300);
    expect(scrollToPage).toHaveBeenCalledWith(2);
  });
});
