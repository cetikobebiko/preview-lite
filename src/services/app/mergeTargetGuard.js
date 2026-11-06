// Birleştirme hedefinin girdi dosyalarından biri olmasını engeller.
// `merge_session_pdfs` yalnız oturumun özel temp kökünü reddeder; hedef açık
// belgenin diskteki yolu ya da bir ek PDF ise sonuç o dosyanın üzerine atomik
// olarak yazılır. Frontend sembolik bağları çözemez; karşılaştırma yalnız
// sözdizimsel normalizasyonla yapılır (bkz. ipc-inventory.md).

const CASE_INSENSITIVE_PLATFORMS = new Set(["macos", "windows"]);

/** Yolu karşılaştırma için normalize eder: NFC, ayırıcılar, `.`/`..`, sondaki ayırıcı, büyük/küçük harf. */
export function comparablePath(path, platform) {
  if (typeof path !== "string" || path.trim() === "") return "";
  let value = path.normalize("NFC");
  const windows = platform === "windows";
  if (windows) value = value.replace(/^\\\\\?\\(UNC\\)?/i, (_, unc) => (unc ? "\\\\" : "")).replace(/\\/g, "/");
  const unc = windows && value.startsWith("//");
  const absolute = value.startsWith("/");
  const segments = [];
  for (const segment of value.split("/")) {
    if (segment === "" || segment === ".") continue;
    if (segment === "..") {
      if (segments.length > 0 && segments[segments.length - 1] !== "..") segments.pop();
      else if (!absolute) segments.push(segment);
      continue;
    }
    segments.push(segment);
  }
  const prefix = unc ? "//" : absolute ? "/" : "";
  const joined = prefix + segments.join("/");
  // Bilinmeyen platformda da harf duyarsız karşılaştır: yanlış pozitif yalnız reddeder.
  return CASE_INSENSITIVE_PLATFORMS.has(platform) || platform === "unknown" || !platform
    ? joined.toLowerCase()
    : joined;
}

/**
 * Hedef, korunan yollardan biriyle aynıysa kullanıcıya gösterilecek hata
 * metnini, değilse `null` döndürür.
 */
export function mergeTargetConflict(targetPath, { documentPath = "", inputPaths = [], platform } = {}) {
  const target = comparablePath(targetPath, platform);
  if (!target) return null;
  if (documentPath && comparablePath(documentPath, platform) === target) {
    return "Birleştirme sonucu açık belgenin kendi dosyasına yazılamaz. Farklı bir dosya adı seçin.";
  }
  if (inputPaths.some((input) => comparablePath(input, platform) === target)) {
    return "Birleştirme sonucu birleştirilen PDF dosyalarından birinin üzerine yazılamaz. Farklı bir dosya adı seçin.";
  }
  return null;
}
