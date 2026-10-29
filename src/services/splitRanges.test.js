import { describe, expect, it } from "vitest";
import { bookmarkRangesText, everyNPagesRangesText } from "./splitRanges.js";

describe("split ranges", () => {
  it("splits every N pages, leaving the remainder last", () => {
    expect(everyNPagesRangesText(7, 3)).toBe("1-3, 4-6, 7");
    expect(everyNPagesRangesText(4, 1)).toBe("1, 2, 3, 4");
    expect(everyNPagesRangesText(2, 5)).toBe("1-2");
    expect(everyNPagesRangesText(5, 0)).toBe("");
  });

  it("splits at top-level bookmarks, keeping front matter and ignoring bad targets", () => {
    const outline = [{ page_index: 4 }, { page_index: 2 }, { page_index: 2 }, { page_index: 99 }, { page_index: -1 }];
    expect(bookmarkRangesText(outline, 8)).toBe("1-2, 3-4, 5-8");
    expect(bookmarkRangesText([{ page_index: 0 }, { page_index: 3 }], 4)).toBe("1-3, 4");
    expect(bookmarkRangesText([], 4)).toBe("");
  });

  it("maps bookmark pages through the display order", () => {
    // Physical page 0 is shown third.
    expect(bookmarkRangesText([{ page_index: 0 }], 3, [1, 2, 0])).toBe("1-2, 3");
  });
});
