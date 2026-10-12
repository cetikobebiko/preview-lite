const SUPPORTED_COLOR_MODES = new Set(["normal", "dark", "invert", "grayscale", "sepia"]);

export function transformRgbaBuffer(data, colorMode = "normal") {
  if (!data || typeof data.length !== "number") {
    throw new Error("Geçersiz görüntü piksel verisi.");
  }

  const normalizedMode = SUPPORTED_COLOR_MODES.has(colorMode) ? colorMode : "normal";

  if (normalizedMode === "normal") {
    // Typed arrays (binary IPC frames) are wrapped without copying the pixels.
    if (data instanceof Uint8ClampedArray) return data;
    if (data instanceof Uint8Array) {
      return new Uint8ClampedArray(data.buffer, data.byteOffset, data.length);
    }
    return new Uint8ClampedArray(data);
  }

  // Color modes transform a single copy so the source buffer stays untouched.
  const pixels = new Uint8ClampedArray(data);

  for (let index = 0; index < pixels.length; index += 4) {
    const red = pixels[index];
    const green = pixels[index + 1];
    const blue = pixels[index + 2];

    if (normalizedMode === "dark" || normalizedMode === "invert") {
      pixels[index] = 255 - red;
      pixels[index + 1] = 255 - green;
      pixels[index + 2] = 255 - blue;
    } else if (normalizedMode === "grayscale") {
      const gray = Math.trunc(0.299 * red + 0.587 * green + 0.114 * blue);
      pixels[index] = gray;
      pixels[index + 1] = gray;
      pixels[index + 2] = gray;
    } else if (normalizedMode === "sepia") {
      pixels[index] = Math.trunc(Math.min(0.393 * red + 0.769 * green + 0.189 * blue, 255));
      pixels[index + 1] = Math.trunc(Math.min(0.349 * red + 0.686 * green + 0.168 * blue, 255));
      pixels[index + 2] = Math.trunc(Math.min(0.272 * red + 0.534 * green + 0.131 * blue, 255));
    }
  }

  return pixels;
}

export function paintBufferToCanvas(canvasObj, buffer, options = {}) {
  if (
    !canvasObj ||
    !buffer ||
    !Number.isFinite(buffer.width) ||
    !Number.isFinite(buffer.height) ||
    buffer.width <= 0 ||
    buffer.height <= 0
  ) {
    throw new Error("Geçersiz görüntü verisi.");
  }

  const safeWidth = Math.floor(buffer.width);
  const safeHeight = Math.floor(buffer.height);
  const expectedLength = safeWidth * safeHeight * 4;
  if (!buffer.data || buffer.data.length !== expectedLength) {
    throw new Error("Eksik görüntü piksel verisi.");
  }

  const ctx = canvasObj.getContext("2d", { alpha: false });
  if (!ctx) {
    throw new Error("Canvas 2D bağlamı oluşturulamadı.");
  }

  const pixels = transformRgbaBuffer(buffer.data, options.colorMode);
  canvasObj.width = safeWidth;
  canvasObj.height = safeHeight;

  ctx.fillStyle = "#FFFFFF";
  ctx.fillRect(0, 0, safeWidth, safeHeight);
  ctx.putImageData(new ImageData(pixels, safeWidth, safeHeight), 0, 0);

  return { width: safeWidth, height: safeHeight };
}
