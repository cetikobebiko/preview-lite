import { describe, expect, it } from "vitest";
import { parsePageRanges, toPhysicalPages, toPhysicalRanges } from "./pageRanges.js";

const ranges = (text, pageCount = 10) => parsePageRanges(text, pageCount, { mode: "ranges" });
const pages = (text, pageCount = 10) => parsePageRanges(text, pageCount, { mode: "pages" });
const sequence = (text, pageCount = 10) => parsePageRanges(text, pageCount, { mode: "sequence" });

describe("parsePageRanges valid input", () => {
  it.each([
    ["1", [[0, 0]]],
    ["10", [[9, 9]]],
    ["1-10", [[0, 9]]],
    ["3-3", [[2, 2]]],
    ["1-3, 5, 8-10", [[0, 2], [4, 4], [7, 9]]],
    ["8-10,1-3", [[7, 9], [0, 2]]],
    ["1-3,2-4", [[0, 2], [1, 3]]],
    ["03", [[2, 2]]],
  ])("ranges %j", (text, expected) => {
    expect(ranges(text)).toEqual({ ok: true, ranges: expected });
  });

  it.each([
    ["1", [0]],
    ["1-3, 5", [0, 1, 2, 4]],
    ["5, 1-3", [0, 1, 2, 4]],
    ["1-3,2-4", [0, 1, 2, 3]],
    ["2,2,2", [1]],
    ["1-10", [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]],
  ])("pages %j", (text, expected) => {
    expect(pages(text)).toEqual({ ok: true, pages: expected });
  });
});

describe("parsePageRanges whitespace tolerance", () => {
  it.each([
    "1-3,5",
    " 1-3 , 5 ",
    "1 - 3, 5",
    "1 -3,5",
    "1- 3,5",
    "\t1-3,\t5\n",
    " 1-3, 5",
  ])("%j", (text) => {
    expect(ranges(text)).toEqual({ ok: true, ranges: [[0, 2], [4, 4]] });
  });

  it("rejects whitespace inside a number", () => {
    expect(ranges("1 0")).toMatchObject({ ok: false, part: "1 0" });
  });
});

describe("parsePageRanges bounds", () => {
  it.each([
    ["0", "0"],
    ["0-3", "0-3"],
    ["11", "11"],
    ["9-11", "9-11"],
    ["1, 11", "11"],
    ["99999999999999999999", "99999999999999999999"],
  ])("%j is out of bounds", (text, part) => {
    const result = ranges(text);
    expect(result).toMatchObject({ ok: false, part });
    expect(result.error).toContain("sınır dışında");
    expect(result.error).toContain("1-10");
  });

  it("accepts the first and last page exactly", () => {
    expect(ranges("1, 10", 10)).toEqual({ ok: true, ranges: [[0, 0], [9, 9]] });
    expect(pages("1", 1)).toEqual({ ok: true, pages: [0] });
  });

  it.each([0, -1, 1.5, NaN, null, "10"])("rejects pageCount %j", (pageCount) => {
    expect(ranges("1", pageCount)).toMatchObject({ ok: false, part: null });
  });
});

describe("parsePageRanges invalid input", () => {
  it.each([
    ["5-3", "5-3", "ters aralık"],
    ["1-3, 10-2", "10-2", "ters aralık"],
    ["1-3-5", "1-3-5", "geçersiz"],
    ["1--3", "1--3", "geçersiz"],
    ["-3", "-3", "geçersiz"],
    ["3-", "3-", "geçersiz"],
    ["2.5", "2.5", "geçersiz"],
    ["3a", "3a", "geçersiz"],
    ["+3", "+3", "geçersiz"],
    ["1e2", "1e2", "geçersiz"],
    ["1–3", "1–3", "geçersiz"],
    ["٣", "٣", "geçersiz"],
    ["1;3", "1;3", "geçersiz"],
    ["abc", "abc", "geçersiz"],
  ])("%j", (text, part, reason) => {
    const result = ranges(text);
    expect(result).toMatchObject({ ok: false, part });
    expect(result.error).toContain(`"${part}"`);
    expect(result.error).toContain(reason);
  });

  it.each([
    [", ,", "1."],
    ["1, ,3", "2."],
    ["1,,3", "2."],
    ["1-3,", "2."],
    [",1", "1."],
  ])("empty part in %j", (text, position) => {
    const result = ranges(text);
    expect(result).toMatchObject({ ok: false, part: "" });
    expect(result.error).toBe(`${position} parça boş (fazladan virgül).`);
  });

  it.each(["", "   ", "\t\n", null, undefined, 5])("empty input %j", (text) => {
    expect(ranges(text)).toEqual({ ok: false, error: "Sayfa aralığı boş.", part: null });
  });

  it("reports the first failing part", () => {
    expect(ranges("2.5, 5-3")).toMatchObject({ ok: false, part: "2.5" });
  });

  it("applies the same rules in pages mode", () => {
    expect(pages("5-3")).toMatchObject({ ok: false, part: "5-3" });
    expect(pages("1, 11")).toMatchObject({ ok: false, part: "11" });
  });

  it("requires a known mode", () => {
    expect(() => parsePageRanges("1", 10)).toThrow(TypeError);
    expect(() => parsePageRanges("1", 10, { mode: "set" })).toThrow(TypeError);
  });
});

describe("parsePageRanges sequence mode", () => {
  it.each([
    ["3, 1", [2, 0]],
    ["4-6, 1-2", [3, 4, 5, 0, 1]],
    [" 2 - 3 ,10", [1, 2, 9]],
  ])("%j keeps input order", (text, expected) => {
    expect(sequence(text)).toEqual({ ok: true, pages: expected });
  });

  it.each([
    ["2, 2", "2", 2],
    ["1-3, 2", "2", 2],
    ["1-3, 3-5", "3-5", 3],
  ])("rejects duplicate page in %j", (text, part, page) => {
    expect(sequence(text)).toEqual({ ok: false, error: `"${part}" sayfa ${page} tekrar ediyor.`, part });
  });

  it("applies the shared syntax and bound rules", () => {
    expect(sequence("5-3")).toMatchObject({ ok: false, part: "5-3" });
    expect(sequence("1,,2")).toMatchObject({ ok: false, part: "" });
    expect(sequence("11")).toMatchObject({ ok: false, part: "11" });
  });
});

describe("display to physical mapping", () => {
  it("is the identity without a reordered pageOrder", () => {
    expect(toPhysicalPages([2, 0], undefined)).toEqual([2, 0]);
    expect(toPhysicalPages([2, 0], [0, 1, 2])).toEqual([2, 0]);
    expect(toPhysicalRanges([[0, 2]], null)).toEqual({ ok: true, ranges: [[0, 2]] });
    expect(toPhysicalRanges([[0, 2]], [0, 1, 2, 3])).toEqual({ ok: true, ranges: [[0, 2]] });
  });

  it("maps display pages through pageOrder, keeping order", () => {
    expect(toPhysicalPages([0, 1, 2], [2, 0, 1])).toEqual([2, 0, 1]);
    // Viewer-only deletion: display 1 is physical 2.
    expect(toPhysicalPages([0, 1], [0, 2, 3])).toEqual([0, 2]);
  });

  it("maps ranges that stay contiguous in the file", () => {
    // Display order 4,5,6,1,2,3 (physical 3,4,5,0,1,2).
    const order = [3, 4, 5, 0, 1, 2];
    expect(toPhysicalRanges([[0, 2], [3, 5], [4, 4]], order)).toEqual({ ok: true, ranges: [[3, 5], [0, 2], [1, 1]] });
  });

  it.each([
    [[[0, 1]], [1, 0, 2], "1-2"],
    [[[0, 1]], [0, 2, 3], "1-2"],
    [[[2, 2], [2, 4]], [0, 1, 2, 4, 3], "3-5"],
  ])("rejects %j split apart by pageOrder %j", (displayRanges, order, part) => {
    const result = toPhysicalRanges(displayRanges, order);
    expect(result).toMatchObject({ ok: false, part });
    expect(result.error).toContain("belgede ardışık değil");
  });
});
