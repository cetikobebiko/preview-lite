<script>
  import { fade, fly } from "svelte/transition";
  import { ask, open, save } from "@tauri-apps/plugin-dialog";
  import {
    exportSessionPagesAsImages,
    exportSessionTextContent,
    printSessionPdf,
  } from "../services/backend/index.js";
  import { exportErrorMessage } from "../services/app/exportErrorMessage.js";
  import {
    outputConflictMessage,
    publishWithOverwriteConfirmation,
  } from "../services/app/outputConflict.js";
  import { parsePageRanges, toPhysicalPages } from "../utils/pageRanges.js";

  let {
    sessionId = "",
    contentGeneration = 0,
    maxPages = 1,
    // Display order (viewer.pageOrder); typed numbers are display positions.
    pageOrder = null,
    onclose,
    onsuccess,
  } = $props();
  
  let pageRange = $state("");
  let exportType = $state("image"); // "image", "text", "print"
  
  // Image options
  let imgFormat = $state("png");
  let imgDpi = $state(300);
  
  // Text options
  let txtFormat = $state("markdown");
  
  let isExporting = $state(false);
  let errorMsg = $state("");

  // Empty input means every page; anything else must parse cleanly or nothing is exported.
  function selectPages() {
      if (!pageRange.trim()) {
          return { ok: true, pages: Array.from({ length: maxPages }, (_, i) => i) };
      }
      return parsePageRanges(pageRange, maxPages, { mode: "pages" });
  }

  async function handleExport() {
      errorMsg = "";
      const selection = selectPages();
      if (!selection.ok) {
          errorMsg = selection.error;
          return;
      }
      // The backend addresses physical PDF pages.
      const selectedPages = toPhysicalPages(selection.pages, pageOrder);
      if (selectedPages.length === 0) {
          errorMsg = "Belgede dışa aktarılacak sayfa yok.";
          return;
      }
      
      isExporting = true;
      
      try {
          if (exportType === "image") {
              const outputDir = await open({
                  directory: true,
                  title: "Görsellerin Kaydedileceği Klasörü Seçin"
              });
              
              if (outputDir) {
                  const result = await publishWithOverwriteConfirmation(
                    (overwrite) =>
                      exportSessionPagesAsImages(
                        sessionId,
                        contentGeneration,
                        selectedPages,
                        outputDir,
                        imgFormat,
                        imgDpi,
                        overwrite,
                      ),
                    (names) =>
                      ask(outputConflictMessage(names), {
                        title: "Dosyalar Zaten Var",
                        kind: "warning",
                        okLabel: "Üzerine Yaz",
                        cancelLabel: "İptal",
                      }),
                  );
                  if (result.status === "declined") {
                      errorMsg = "Dışa aktarma iptal edildi; mevcut dosyalara dokunulmadı.";
                      return;
                  }
                  onsuccess?.("Görseller başarıyla dışa aktarıldı!");
                  onclose?.();
              }
          } else if (exportType === "text") {
              const savePath = await save({
                  title: "Metin Olarak Kaydet",
                  defaultPath: `disa_aktarim.${txtFormat === "markdown" ? "md" : "txt"}`,
                  filters: [{ name: "Text File", extensions: ["txt", "md"] }]
              });
              
              if (savePath) {
                  const textContent = await exportSessionTextContent(
                    sessionId,
                    contentGeneration,
                    selectedPages,
                    txtFormat,
                  );
                  const { writeTextFile } = await import('@tauri-apps/plugin-fs');
                  await writeTextFile(savePath, textContent);
                  onsuccess?.("Metin başarıyla dışa aktarıldı!");
                  onclose?.();
              }
          } else if (exportType === "print") {
              const ranges = pageRange.trim() ? selectedPages : null;
              await printSessionPdf(sessionId, contentGeneration, ranges);
              onsuccess?.("Yazdırma işlemi başlatıldı!");
              onclose?.();
          }
      } catch (err) {
          errorMsg = exportErrorMessage(err);
      } finally {
          isExporting = false;
      }
  }
</script>

<dialog
  open
  class="modal-overlay"
  transition:fade={{ duration: 150 }}
  oncancel={(event) => {
    event.preventDefault();
    onclose?.();
  }}
>
  <div class="modal-content glass-panel" transition:fly={{ y: 20, duration: 250 }}>
      <div class="modal-header">
          <h3>Dışa Aktar (Export)</h3>
          <button type="button" class="close-btn" aria-label="Kapat" onclick={() => onclose?.()}>×</button>
      </div>
      
      <div class="modal-body">
          {#if errorMsg}
              <div class="error-msg" role="alert">{errorMsg}</div>
          {/if}
          
          <div class="form-group">
              <label for="export-page-range">Sayfa Aralığı (Tümü için boş bırakın)</label>
              <input id="export-page-range" type="text" bind:value={pageRange} placeholder="Örn: 1-5, 8, 11-13" class="tech-input" />
              <small class="hint">Toplam Sayfa: {maxPages}</small>
          </div>
          
          <div class="form-group type-tabs">
              <button type="button" class="tab-btn {exportType === 'print' ? 'active' : ''}" onclick={() => exportType = 'print'}>Yazdır</button>
              <button type="button" class="tab-btn {exportType === 'image' ? 'active' : ''}" onclick={() => exportType = 'image'}>Resim (Görsel)</button>
              <button type="button" class="tab-btn {exportType === 'text' ? 'active' : ''}" onclick={() => exportType = 'text'}>Metin (StExt)</button>
          </div>
          
          <div class="export-options">
              {#if exportType === 'image'}
                  <div class="form-group">
                      <label for="export-image-format">Format</label>
                      <select id="export-image-format" bind:value={imgFormat} class="tech-input">
                          <option value="png">PNG (Yüksek Kalite)</option>
                          <option value="jpeg">JPEG (Daha Küçük Boyut)</option>
                      </select>
                  </div>
                  <div class="form-group">
                      <label for="export-image-dpi">Çözünürlük (DPI)</label>
                      <select id="export-image-dpi" bind:value={imgDpi} class="tech-input">
                          <option value={150}>150 DPI (Standart Okuma)</option>
                          <option value={300}>300 DPI (Baskı Kalitesi)</option>
                          <option value={600}>600 DPI (Yüksek Detay)</option>
                      </select>
                  </div>
              {:else if exportType === 'text'}
                  <div class="form-group">
                      <label for="export-text-format">Metin Formatı</label>
                      <select id="export-text-format" bind:value={txtFormat} class="tech-input">
                          <option value="markdown">Markdown (.md)</option>
                          <option value="txt">Düz Metin (.txt)</option>
                      </select>
                  </div>
              {:else}
                  <div style="font-size: 0.9rem; color: var(--text-muted); text-align: center;">
                      Belge doğrudan işletim sisteminin varsayılan yazdırma kuyruğuna gönderilecektir.
                  </div>
              {/if}
          </div>
          
          <button type="button" class="submit-btn" disabled={isExporting} onclick={handleExport}>
              {isExporting ? 'İşleniyor...' : (exportType === 'print' ? 'Yazdır' : 'Dışa Aktar')}
          </button>
      </div>
  </div>
</dialog>

<style>
  .modal-overlay {
      position: fixed; top: 0; left: 0; right: 0; bottom: 0;
      background: rgba(0, 0, 0, 0.6);
      backdrop-filter: blur(8px);
      display: flex; align-items: center; justify-content: center;
      z-index: 10000;
  }
  
  .modal-content {
      background: var(--bg-surface);
      border: 1px solid var(--border-color);
      border-radius: 12px;
      width: 400px;
      max-width: 90vw;
      box-shadow: 0 10px 30px rgba(0,0,0,0.5);
      overflow: hidden;
  }
  
  .modal-header {
      display: flex; justify-content: space-between; align-items: center;
      padding: 16px 20px; border-bottom: 1px solid var(--border-color);
      background: var(--bg-surface-hover);
  }
  
  .modal-header h3 { margin: 0; font-size: 1.1rem; color: var(--text-primary); }
  
  .close-btn {
      background: none; border: none; color: var(--text-secondary);
      font-size: 1.5rem; cursor: pointer; transition: color 0.2s;
  }
  .close-btn:hover { color: var(--text-primary); }
  
  .modal-body { padding: 20px; }
  
  .form-group { margin-bottom: 16px; }
  
  .form-group label {
      display: block; margin-bottom: 6px;
      font-size: 0.85rem; color: var(--text-secondary); text-transform: uppercase; letter-spacing: 0.5px;
  }
  
  .tech-input {
      width: 100%; padding: 10px; background: rgba(0,0,0,0.2);
      border: 1px solid var(--border-color); border-radius: 6px;
      color: var(--text-primary); font-family: inherit; font-size: 0.95rem;
      transition: all 0.2s;
  }
  .tech-input:focus { outline: none; border-color: var(--accent-color); box-shadow: 0 0 0 2px rgba(138, 43, 226, 0.2); }
  
  .hint { display: block; margin-top: 4px; font-size: 0.75rem; color: var(--text-muted); }
  
  .type-tabs { display: flex; gap: 8px; margin-bottom: 20px; }
  
  .tab-btn {
      flex: 1; padding: 10px; background: rgba(0,0,0,0.2);
      border: 1px solid var(--border-color); border-radius: 6px;
      color: var(--text-secondary); cursor: pointer; transition: all 0.2s;
  }
  .tab-btn.active { background: var(--accent-color); color: white; border-color: var(--accent-color); }
  
  .export-options {
      padding: 16px; background: rgba(255,255,255,0.03);
      border-radius: 8px; margin-bottom: 20px;
  }
  
  .submit-btn {
      width: 100%; padding: 12px; background: var(--accent-color);
      color: white; border: none; border-radius: 6px; font-weight: 600;
      cursor: pointer; transition: all 0.2s;
  }
  .submit-btn:hover:not(:disabled) { background: var(--accent-hover); box-shadow: 0 0 15px var(--accent-color); }
  .submit-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  
  .error-msg {
      background: rgba(255, 59, 48, 0.1); border: 1px solid rgba(255, 59, 48, 0.3);
      color: #ff3b30; padding: 10px; border-radius: 6px; margin-bottom: 16px;
      font-size: 0.9rem; text-align: center;
  }
</style>
