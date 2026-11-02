// Meta veri, sayfa etiketi ve yer imi düzenleyicilerinin saf yardımcıları. Yazım ve asıl
// doğrulama Rust'tadır (document_structure); buradaki biçimlendirme Rust'ın döndürdüğü aralıkları
// önizleme ve sayfa göstergesi için metne çevirir, PDF ayrıştırmaz.

export const PAGE_LABEL_STYLES = Object.freeze([
  Object.freeze({ value: "decimal", label: "1, 2, 3" }),
  Object.freeze({ value: "lower_roman", label: "i, ii, iii" }),
  Object.freeze({ value: "upper_roman", label: "I, II, III" }),
  Object.freeze({ value: "lower_alpha", label: "a, b, c" }),
  Object.freeze({ value: "upper_alpha", label: "A, B, C" }),
  Object.freeze({ value: "none", label: "Numara yok (yalnız önek)" }),
]);

const ROMAN = [
  [1000, "m"], [900, "cm"], [500, "d"], [400, "cd"], [100, "c"], [90, "xc"],
  [50, "l"], [40, "xl"], [10, "x"], [9, "ix"], [5, "v"], [4, "iv"], [1, "i"],
];

function roman(n) {
  let out = "";
  for (const [value, part] of ROMAN) {
    out += part.repeat(Math.floor(n / value));
    n %= value;
  }
  return out;
}

function alpha(n) {
  const index = Math.max(1, n) - 1;
  return String.fromCharCode(97 + (index % 26)).repeat(Math.floor(index / 26) + 1);
}

/** The label `range` gives `page` (0-based), as PDF readers number it. */
export function formatPageLabel(range, page) {
  const number = range.style === "none"
    ? range.firstNumber
    : range.firstNumber + (page - range.startPage);
  const numeral = {
    decimal: () => String(number),
    lower_roman: () => roman(number),
    upper_roman: () => roman(number).toUpperCase(),
    lower_alpha: () => alpha(number),
    upper_alpha: () => alpha(number).toUpperCase(),
    none: () => "",
  }[range.style]?.() ?? "";
  return `${range.prefix ?? ""}${numeral}`;
}

/**
 * Each physical page's label, indexed by 0-based page; empty when the document has no
 * usable labels. A page whose label is empty (no numbering, no prefix) gets "".
 */
export function pageLabelsFor(ranges, pageCount) {
  if (!Array.isArray(ranges) || ranges.length === 0 || pageLabelRangesError(ranges, pageCount)) return [];
  const labels = [];
  let rangeIndex = 0;
  for (let page = 0; page < pageCount; page += 1) {
    while (rangeIndex + 1 < ranges.length && ranges[rangeIndex + 1].startPage <= page) rangeIndex += 1;
    labels.push(formatPageLabel(ranges[rangeIndex], page));
  }
  return labels;
}

/** Problems with `ranges` for a document of `pageCount` pages, or null. */
export function pageLabelRangesError(ranges, pageCount) {
  if (ranges.length === 0) return null;
  if (ranges[0].startPage !== 0) return "İlk aralık 1. sayfadan başlamalı.";
  for (let index = 0; index < ranges.length; index += 1) {
    const range = ranges[index];
    if (!Number.isInteger(range.startPage) || range.startPage < 0 || range.startPage >= pageCount) {
      return `Aralık ${index + 1}: başlangıç sayfası 1 ile ${pageCount} arasında olmalı.`;
    }
    if (index > 0 && range.startPage <= ranges[index - 1].startPage) {
      return `Aralık ${index + 1}: başlangıç sayfası öncekinden büyük olmalı.`;
    }
    if (!Number.isInteger(range.firstNumber) || range.firstNumber < 1) {
      return `Aralık ${index + 1}: ilk numara en az 1 olmalı.`;
    }
  }
  return null;
}

let nextKey = 0;
const newKey = () => `outline-${(nextKey += 1)}`;

/** PDFium's outline (`get_session_pdf_outline`) as an editable tree that remembers each item's source. */
export function outlineToEditTree(outline, path = []) {
  return (outline ?? []).map((node, index) => {
    const nodePath = [...path, index];
    return {
      key: newKey(),
      title: node.title,
      source: { path: nodePath, title: node.title },
      pageIndex: null,
      displayPage: node.page_index,
      children: outlineToEditTree(node.children, nodePath),
    };
  });
}

/** A new bookmark to `pageIndex`. */
export function newOutlineNode(title, pageIndex) {
  return { key: newKey(), title, source: null, pageIndex, displayPage: pageIndex, children: [] };
}

/** The tree in the shape `set_session_outline` takes. */
export function outlineEditTreeToWire(tree) {
  return tree.map((node) => ({
    title: node.title.trim(),
    source: node.source,
    pageIndex: node.pageIndex,
    children: outlineEditTreeToWire(node.children),
  }));
}

/** The first empty title in `tree`, as a path, or null. */
export function findEmptyOutlineTitle(tree, path = []) {
  for (let index = 0; index < tree.length; index += 1) {
    if (!tree[index].title.trim()) return [...path, index];
    const inner = findEmptyOutlineTitle(tree[index].children, [...path, index]);
    if (inner) return inner;
  }
  return null;
}

function siblingsAt(tree, path) {
  let list = tree;
  for (const index of path.slice(0, -1)) list = list[index].children;
  return list;
}

/** `tree` with the node at `path` changed by `update` (which returns the new node). */
export function updateOutlineNode(tree, path, update) {
  const [head, ...rest] = path;
  return tree.map((node, index) => {
    if (index !== head) return node;
    if (rest.length === 0) return update(node);
    return { ...node, children: updateOutlineNode(node.children, rest, update) };
  });
}

function withSiblings(tree, path, change) {
  if (path.length === 1) return change(tree);
  return updateOutlineNode(tree, path.slice(0, -1), (parent) => ({
    ...parent,
    children: change(parent.children),
  }));
}

export function removeOutlineNode(tree, path) {
  const at = path[path.length - 1];
  return withSiblings(tree, path, (list) => list.filter((_, index) => index !== at));
}

/** Insert `node` as child `index` of `parentPath` (`[]` = top level). */
export function insertOutlineNode(tree, parentPath, index, node) {
  const insert = (list) => [...list.slice(0, index), node, ...list.slice(index)];
  if (parentPath.length === 0) return insert(tree);
  return updateOutlineNode(tree, parentPath, (parent) => ({ ...parent, children: insert(parent.children) }));
}

/** Swap the node at `path` with its previous (`-1`) or next (`+1`) sibling; the node's new path. */
export function moveOutlineNode(tree, path, direction) {
  const at = path[path.length - 1];
  const target = at + direction;
  if (target < 0 || target >= siblingsAt(tree, path).length) return { tree, path };
  const next = withSiblings(tree, path, (list) => {
    const copy = [...list];
    [copy[at], copy[target]] = [copy[target], copy[at]];
    return copy;
  });
  return { tree: next, path: [...path.slice(0, -1), target] };
}

/** Make the node at `path` the last child of its previous sibling; the node's new path. */
export function indentOutlineNode(tree, path) {
  const at = path[path.length - 1];
  if (at === 0) return { tree, path };
  const node = siblingsAt(tree, path)[at];
  const previousPath = [...path.slice(0, -1), at - 1];
  const childCount = siblingsAt(tree, path)[at - 1].children.length;
  const removed = removeOutlineNode(tree, path);
  return {
    tree: insertOutlineNode(removed, previousPath, childCount, node),
    path: [...previousPath, childCount],
  };
}

/** Move the node at `path` out of its parent, right after it; the node's new path. */
export function outdentOutlineNode(tree, path) {
  if (path.length < 2) return { tree, path };
  const node = siblingsAt(tree, path)[path[path.length - 1]];
  const parentPath = path.slice(0, -1);
  const removed = removeOutlineNode(tree, path);
  const index = parentPath[parentPath.length - 1] + 1;
  return {
    tree: insertOutlineNode(removed, parentPath.slice(0, -1), index, node),
    path: [...parentPath.slice(0, -1), index],
  };
}

export function getOutlineNode(tree, path) {
  let node = null;
  let list = tree;
  for (const index of path) {
    node = list[index];
    if (!node) return null;
    list = node.children;
  }
  return node;
}
