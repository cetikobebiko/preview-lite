export const BIONIC_EMPTY_MESSAGE = "Metin yüklenmedi...";
export const BIONIC_BOLD_RATIO = 0.4;

const graphemeSegmenter =
  typeof Intl !== "undefined" && typeof Intl.Segmenter === "function"
    ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
    : null;

// Splits a word into user-perceived characters so emoji (surrogate pairs, ZWJ
// sequences) and combining marks are never cut in half. Falls back to code
// points when Intl.Segmenter is unavailable.
function splitCharacters(word) {
  if (graphemeSegmenter) {
    return Array.from(graphemeSegmenter.segment(word), (part) => part.segment);
  }
  return Array.from(word);
}

export function splitBionicWord(word) {
  const characters = splitCharacters(word);
  if (characters.length <= 1) return { bold: word, rest: "" };
  const mid = Math.ceil(characters.length * BIONIC_BOLD_RATIO);
  return {
    bold: characters.slice(0, mid).join(""),
    rest: characters.slice(mid).join(""),
  };
}

// Returns plain data (lines -> words -> {bold, rest}); callers must render the
// strings as text nodes, never as HTML.
export function toBionicLines(rawText) {
  if (!rawText) return [];
  return String(rawText)
    .split("\n")
    .map((line) => line.split(" ").map(splitBionicWord));
}
