import { describe, expect, it } from "vitest";
import {
  findEmptyOutlineTitle,
  formatPageLabel,
  indentOutlineNode,
  insertOutlineNode,
  moveOutlineNode,
  newOutlineNode,
  outdentOutlineNode,
  outlineEditTreeToWire,
  outlineToEditTree,
  pageLabelRangesError,
  pageLabelsFor,
  removeOutlineNode,
} from "./documentStructure.js";

const range = (startPage, style, prefix = "", firstNumber = 1) => ({ startPage, style, prefix, firstNumber });

describe("page labels", () => {
  it("formats every PDF numbering style", () => {
    expect(formatPageLabel(range(0, "decimal"), 4)).toBe("5");
    expect(formatPageLabel(range(0, "lower_roman"), 13)).toBe("xiv");
    expect(formatPageLabel(range(2, "upper_roman", "", 1994), 2)).toBe("MCMXCIV");
    expect(formatPageLabel(range(0, "lower_alpha"), 27)).toBe("bb");
    expect(formatPageLabel(range(0, "upper_alpha"), 0)).toBe("A");
    expect(formatPageLabel(range(5, "decimal", "Ek-", 3), 6)).toBe("Ek-4");
    expect(formatPageLabel(range(5, "none", "Kapak", 9), 7)).toBe("Kapak");
  });

  it("reports ranges the backend would refuse", () => {
    expect(pageLabelRangesError([], 3)).toBeNull();
    expect(pageLabelRangesError([range(0, "decimal"), range(2, "lower_roman")], 3)).toBeNull();
    expect(pageLabelRangesError([range(1, "decimal")], 3)).toMatch(/1\. sayfadan/);
    expect(pageLabelRangesError([range(0, "decimal"), range(0, "decimal")], 3)).toMatch(/öncekinden büyük/);
    expect(pageLabelRangesError([range(0, "decimal"), range(3, "decimal")], 3)).toMatch(/1 ile 3/);
    expect(pageLabelRangesError([range(0, "decimal", "", 0)], 3)).toMatch(/en az 1/);
  });

  it("labels every physical page from its range, or none when the ranges are unusable", () => {
    expect(pageLabelsFor([range(0, "lower_roman"), range(1, "decimal")], 3)).toEqual(["i", "1", "2"]);
    expect(pageLabelsFor([range(0, "none", "Kapak"), range(1, "upper_alpha", "Ek-", 2)], 3)).toEqual(["Kapak", "Ek-B", "Ek-C"]);
    expect(pageLabelsFor([range(0, "none")], 2)).toEqual(["", ""]);
    expect(pageLabelsFor([], 3)).toEqual([]);
    expect(pageLabelsFor(null, 3)).toEqual([]);
    expect(pageLabelsFor([range(0, "decimal"), range(5, "decimal")], 3)).toEqual([]);
  });
});

describe("outline editing", () => {
  const outline = [
    { title: "A", page_index: 0, children: [{ title: "A1", page_index: 1, children: [] }] },
    { title: "B", page_index: 2, children: [] },
  ];
  const titles = (tree) => tree.map((node) => [node.title, titles(node.children)]);

  it("remembers where each existing bookmark came from", () => {
    const wire = outlineEditTreeToWire(outlineToEditTree(outline));
    expect(wire).toEqual([
      {
        title: "A",
        source: { path: [0], title: "A" },
        pageIndex: null,
        children: [{ title: "A1", source: { path: [0, 0], title: "A1" }, pageIndex: null, children: [] }],
      },
      { title: "B", source: { path: [1], title: "B" }, pageIndex: null, children: [] },
    ]);
  });

  it("moves, nests and unnests bookmarks and returns their new paths", () => {
    let tree = outlineToEditTree(outline);
    let result = moveOutlineNode(tree, [1], -1);
    expect(titles(result.tree)).toEqual([["B", []], ["A", [["A1", []]]]]);
    expect(result.path).toEqual([0]);
    // At the edge nothing moves.
    expect(moveOutlineNode(tree, [0], -1).tree).toBe(tree);

    result = indentOutlineNode(tree, [1]);
    expect(titles(result.tree)).toEqual([["A", [["A1", []], ["B", []]]]]);
    expect(result.path).toEqual([0, 1]);

    result = outdentOutlineNode(tree, [0, 0]);
    expect(titles(result.tree)).toEqual([["A", []], ["A1", []], ["B", []]]);
    expect(result.path).toEqual([1]);

    tree = insertOutlineNode(tree, [0], 1, newOutlineNode("Yeni", 2));
    expect(titles(tree)).toEqual([["A", [["A1", []], ["Yeni", []]]], ["B", []]]);
    expect(outlineEditTreeToWire(tree)[0].children[1]).toEqual({ title: "Yeni", source: null, pageIndex: 2, children: [] });

    tree = removeOutlineNode(tree, [0]);
    expect(titles(tree)).toEqual([["B", []]]);
  });

  it("finds an empty title anywhere in the tree", () => {
    const tree = outlineToEditTree(outline);
    expect(findEmptyOutlineTitle(tree)).toBeNull();
    tree[0].children[0].title = "  ";
    expect(findEmptyOutlineTitle(tree)).toEqual([0, 0]);
  });
});
