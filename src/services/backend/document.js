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
