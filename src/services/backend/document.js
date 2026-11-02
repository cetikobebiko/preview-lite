import { callBackend } from "./client.js";

export const readImageFile = (path, rotation, flipH, flipV) =>
  callBackend("readImageFile", { path, rotation, flipH, flipV });

export const convertAndSaveImage = (
  sourcePath,
  targetPath,
  format,
  rotation,
  flipH,
  flipV,
) =>
  callBackend("convertAndSaveImage", {
    sourcePath,
    targetPath,
    format,
    rotation,
    flipH,
    flipV,
  });

// One page per image; `options` is `{ pageSize: "image" | "a4" | "letter", margin }` (points).
export const createPdfFromImages = (sourcePaths, targetPath, options) =>
  callBackend("createPdfFromImages", { sourcePaths, targetPath, options });
