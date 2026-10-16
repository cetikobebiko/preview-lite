/**
 * Parses a user-typed page range such as "1-3, 5, 8-10".
 *
 * Grammar (whitespace = anything String.prototype.trim removes):
 *   input := part ("," part)*
 *   part  := number | number "-" number      (whitespace allowed around the part and the dash)
 *   number:= /^\d+$/                          (ASCII digits only; leading zeros allowed: "03" = 3)
 *
 * Rules:
 * - Only whole numbers: "2.5", "3a", "+3", "1e2", "٣" are rejected.
 * - A range uses exactly one hyphen: "1-3-5", "-3", "3-" and "1--3" are rejected;
 *   en/em dashes are not hyphens.
 * - Pages are 1-based; 0 and anything above `pageCount` is out of bounds.
 * - A reversed range ("5-3") is an error; it is never swapped.
 * - Empty input and empty parts (", ,", "1,", ",1") are errors.
 * - Every error names the offending part, so the UI can show it as-is.
 *
 * Page numbers are 1-based *display* positions (viewer.pageOrder), so the result is a
 * 0-based display index. The backend wants physical PDF page indices: convert with
 * `toPhysicalPages` / `toPhysicalRanges` before sending.
 *
 * Modes:
 * - "pages":  `{ ok: true, pages }` — sorted, de-duplicated 0-based page indices
 *             (overlapping parts merge). Used by export/print.
 * - "ranges": `{ ok: true, ranges }` — `[start, end]` 0-based inclusive pairs in input
 *             order; a single page `n` becomes `[n-1, n-1]`. Overlaps are kept because
 *             each range is its own output. Used by split.
 * - "sequence": `{ ok: true, pages }` — 0-based page indices in input order, ranges
 *             expanded; a page appearing twice is an error (the extract backend rejects
 *             duplicates). Used by page extraction, where order is the output order.
 * Failure in any mode: `{ ok: false, error, part }`, `part` being the trimmed
 * offending text (or null when the whole input is at fault).
 */

const PART_PATTERN = /^(\d+)(?:\s*-\s*(\d+))?$/;
const MODES = new Set(["pages", "ranges", "sequence"]);

function failure(error, part = null) {
  return { ok: false, error, part };
}

export function parsePageRanges(text, pageCount, { mode } = {}) {
  if (!MODES.has(mode)) throw new TypeError(`parsePageRanges: bilinmeyen mode: ${mode}`);
  if (!Number.isSafeInteger(pageCount) || pageCount < 1) return failure("Belgede sayfa yok.");

  const input = typeof text === "string" ? text : "";
  if (!input.trim()) return failure("Sayfa aralığı boş.");

  const ranges = [];
  const parts = input.split(",");
  for (let index = 0; index < parts.length; index += 1) {
    const part = parts[index].trim();
    if (!part) return failure(`${index + 1}. parça boş (fazladan virgül).`, part);

    const match = PART_PATTERN.exec(part);
    if (!match) {
      return failure(`"${part}" geçersiz. Tek sayfa (5) veya tek tireli aralık (1-3) yazın; yalnız tam sayılar.`, part);
    }
    const start = Number(match[1]);
    const end = match[2] === undefined ? start : Number(match[2]);
    if (start > end) return failure(`"${part}" ters aralık; başlangıç bitişten büyük olamaz.`, part);
    if (start < 1 || end > pageCount) return failure(`"${part}" sınır dışında; sayfalar 1-${pageCount} arasında olmalı.`, part);
    ranges.push([start - 1, end - 1]);
  }

  if (mode === "ranges") return { ok: true, ranges };

  if (mode === "sequence") {
    const pages = [];
    const seen = new Set();
    for (let index = 0; index < ranges.length; index += 1) {
      const [start, end] = ranges[index];
      for (let page = start; page <= end; page += 1) {
        if (seen.has(page)) return failure(`"${parts[index].trim()}" sayfa ${page + 1} tekrar ediyor.`, parts[index].trim());
        seen.add(page);
        pages.push(page);
      }
    }
    return { ok: true, pages };
  }

  const pages = new Set();
  for (const [start, end] of ranges) for (let page = start; page <= end; page += 1) pages.add(page);
  return { ok: true, pages: [...pages].sort((a, b) => a - b) };
}

function isIdentityOrder(pageOrder) {
  return !Array.isArray(pageOrder) || pageOrder.every((physical, display) => physical === display);
}

/** Maps 0-based display indices to physical PDF page indices; no pageOrder = identity. */
export function toPhysicalPages(pages, pageOrder) {
  if (isIdentityOrder(pageOrder)) return [...pages];
  return pages.map((display) => pageOrder[display]);
}

/**
 * Maps display ranges to physical `[start, end]` ranges. A range that is contiguous on
 * screen but not in the file (pages reordered or hidden only in the viewer) cannot be
 * expressed as one physical range, so it fails instead of exporting other pages.
 */
export function toPhysicalRanges(ranges, pageOrder) {
  if (isIdentityOrder(pageOrder)) return { ok: true, ranges: ranges.map(([start, end]) => [start, end]) };
  const physicalRanges = [];
  for (const [start, end] of ranges) {
    for (let display = start + 1; display <= end; display += 1) {
      if (pageOrder[display] !== pageOrder[display - 1] + 1) {
        const part = `${start + 1}-${end + 1}`;
        return failure(`"${part}" ekranda ardışık ama belgede ardışık değil; önce sayfa sırasını kaydedin.`, part);
      }
    }
    physicalRanges.push([pageOrder[start], pageOrder[end]]);
  }
  return { ok: true, ranges: physicalRanges };
}
