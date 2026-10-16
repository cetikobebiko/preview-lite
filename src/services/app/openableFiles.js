/** Extensions the open dialog offers and the drag-drop flow accepts. */
export const OPENABLE_FILE_EXTENSIONS = Object.freeze(["pdf", "png", "jpg", "jpeg", "webp", "bmp", "svg"]);

export function fileBaseName(path) {
  return String(path).split(/[\\/]/).pop() ?? "";
}

export function isOpenableFilePath(path) {
  if (typeof path !== "string") return false;
  const name = fileBaseName(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 && OPENABLE_FILE_EXTENSIONS.includes(name.slice(dot + 1).toLowerCase());
}
