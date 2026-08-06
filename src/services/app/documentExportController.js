/** Coordinates output-file dialogs, native exports, and stale session-result protection. */
export function createDocumentExportController({
  getContext, saveFile, requestInput, exportPage, extractPages, convertImage, burnAnnotations,
  guard, featureContext, reportUnavailable, onToast, onError,
}) {
  function isCurrent(context) {
    const current = getContext();
    return current.sessionId === context.sessionId && current.contentGeneration === context.contentGeneration && current.imagePath === context.imagePath;
  }

  function baseName(fileName) {
    const index = fileName.lastIndexOf(".");
    return index > 0 ? fileName.substring(0, index) : fileName;
  }

  async function exportCurrentPageImage() {
    const context = getContext();
    if (!context.sessionId) return;
    try {
      const savePath = await saveFile({
        title: `Sayfa ${context.pageIndex + 1} Görsel/SVG Olarak Aktar`,
        defaultPath: `${baseName(context.fileName)}_sayfa_${context.pageIndex + 1}.svg`,
        filters: [{ name: "Vektörel SVG (*.svg)", extensions: ["svg"] }, { name: "PNG Görsel (*.png)", extensions: ["png"] }, { name: "JPEG Görsel (*.jpg)", extensions: ["jpg"] }, { name: "WebP Görsel (*.webp)", extensions: ["webp"] }],
      });
      if (!savePath) return;
      const extension = savePath.split(".").pop()?.toLowerCase() || "svg";
      await exportPage(context.sessionId, context.contentGeneration, context.pageIndex, savePath, extension);
      if (isCurrent(context)) onToast(`Sayfa ${context.pageIndex + 1} ${extension.toUpperCase()} olarak aktarıldı!`);
    } catch (error) {
      console.error("Dışa aktarma hatası:", error);
      onToast("Dışa aktarma hatası: " + error.toString());
    }
  }

  async function exportPdfStructure() {
    const context = getContext();
    if (!context.isPdf) return exportImage();
    if (!context.sessionId) return;
    try {
      const savePath = await saveFile({
        title: "Düzenlenen PDF Belgesini Dışa Aktar", defaultPath: `${baseName(context.fileName)}_düzenlendi.pdf`,
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });
      if (!savePath) return;
      await extractPages(context.sessionId, context.contentGeneration, context.pageOrder, savePath);
      if (isCurrent(context)) onToast("Yeni PDF belgesi başarıyla dışa aktarıldı!");
    } catch (error) {
      console.error("Dışa aktarma hatası:", error);
      if (isCurrent(context)) onError("Dışa aktarma başarısız: " + error.toString());
    }
  }

  async function exportImage() {
    const context = getContext();
    if (!context.imagePath) return;
    try {
      const savePath = await saveFile({
        title: "Farklı Kaydet ve Format Dönüştür", defaultPath: `${baseName(context.fileName)}_dönüştürüldü`,
        filters: [{ name: "WebP Görsel (*.webp)", extensions: ["webp"] }, { name: "PNG Görsel (*.png)", extensions: ["png"] }, { name: "JPEG Görsel (*.jpg)", extensions: ["jpg"] }, { name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });
      if (!savePath) return;
      const extension = savePath.split(".").pop()?.toLowerCase() || "png";
      await convertImage(context.imagePath, savePath, extension, context.rotation, context.flipH, context.flipV);
      if (isCurrent(context)) onToast(`Dosya başarıyla ${extension.toUpperCase()} formatında kaydedildi!`);
    } catch (error) {
      console.error("Dönüştürme hatası:", error);
      if (isCurrent(context)) onError("Dönüştürme başarısız: " + error.toString());
    }
  }

  function parsePageIndices(input, pageCount) {
    const pageIndices = [];
    for (const part of input.split(",")) {
      const trimmed = part.trim();
      if (trimmed.includes("-")) {
        const [start, end] = trimmed.split("-").map((value) => Number.parseInt(value, 10));
        if (Number.isInteger(start) && Number.isInteger(end)) for (let page = start; page <= end; page += 1) if (page >= 1 && page <= pageCount) pageIndices.push(page - 1);
      } else {
        const page = Number.parseInt(trimmed, 10);
        if (Number.isInteger(page) && page >= 1 && page <= pageCount) pageIndices.push(page - 1);
      }
    }
    return pageIndices;
  }

  async function splitPdf() {
    const input = await requestInput({ title: "Sayfaları Dışa Aktar", label: "Sayfa numaraları", placeholder: "Örn: 1, 2, 5 veya 1-3" });
    if (!input) return;
    const context = getContext();
    if (!context.sessionId) return;
    const pageIndices = parsePageIndices(input, context.pageOrder.length);
    if (pageIndices.length === 0) { onToast("Geçerli sayfa numarası girilmedi!"); return; }
    try {
      const savePath = await saveFile({
        title: "Bölünen Sayfaları Yeni PDF Olarak Kaydet", defaultPath: `${baseName(context.fileName)}_Bölünmüş.pdf`,
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });
      if (!savePath) return;
      await extractPages(context.sessionId, context.contentGeneration, pageIndices, savePath);
      if (isCurrent(context)) onToast(`${pageIndices.length} sayfa yeni PDF belgesine bölündü!`);
    } catch (error) {
      console.error("Bölme hatası:", error);
      if (isCurrent(context)) onError("Sayfa bölme başarısız: " + error.toString());
    }
  }

  async function burnInPdf() {
    try { return await guard("annotation_burn_export", featureContext, executeBurnIn); }
    catch (error) { if (!reportUnavailable(error)) throw error; }
  }

  async function executeBurnIn() {
    const context = getContext();
    if (!context.isLoadedPdfDocument || !context.sessionId) return;
    try {
      const savePath = await saveFile({
        title: "Açıklamalı PDF'i Kaydet", defaultPath: `${baseName(context.fileName)}_aciklamali.pdf`,
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });
      if (!savePath) return;
      await burnAnnotations(context.sessionId, context.contentGeneration, JSON.stringify(context.annotations), savePath);
      if (isCurrent(context)) onToast("PDF başarıyla kaydedildi!");
    } catch (error) {
      console.error("Kayıt hatası:", error);
      if (isCurrent(context)) onError("Kayıt başarısız: " + error.toString());
    }
  }

  return Object.freeze({ exportCurrentPageImage, exportPdfStructure, exportImage, splitPdf, burnInPdf });
}
