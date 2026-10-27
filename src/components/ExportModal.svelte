<script>
  import AppDialog from "./AppDialog.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { ask, open, save } from "@tauri-apps/plugin-dialog";
  import {
    exportSessionDocument,
    exportSessionPagesAsImages,
    exportSessionTextToFile,
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
  let exportType = $state("image"); // "image", "text", "document", "print"
  
  // Image options
  let imgFormat = $state("png");
  let imgDpi = $state(300);
  
  // Text options
  let txtFormat = $state("markdown");

  // Document options: Word, PowerPoint, HTML or RTF, written by the job worker.
  const DOCUMENT_FORMATS = {
    docx: { label: "Word (.docx)", filter: "Word" },
    pptx: { label: "PowerPoint (.pptx)", filter: "PowerPoint" },
    html: { label: "Web sayfası (.html)", filter: "HTML" },
    rtf: { label: "Zengin metin (.rtf)", filter: "RTF" },
  };
  let docFormat = $state("docx");
  
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
                  await exportSessionTextToFile(
                    sessionId,
                    contentGeneration,
                    selectedPages,
                    txtFormat,
                    savePath,
                  );
                  onsuccess?.("Metin başarıyla dışa aktarıldı!");
                  onclose?.();
              }
          } else if (exportType === "document") {
              const { filter } = DOCUMENT_FORMATS[docFormat];
              const savePath = await save({
                  title: "Belge Olarak Kaydet",
                  defaultPath: `disa_aktarim.${docFormat}`,
                  filters: [{ name: filter, extensions: [docFormat] }]
              });

              if (savePath) {
                  await exportSessionDocument(
                    sessionId,
                    contentGeneration,
                    selectedPages,
                    docFormat,
                    savePath,
                  );
                  onsuccess?.("Belge başarıyla dışa aktarıldı!");
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

<AppDialog title="Dışa Aktar (Export)" dismissible={!isExporting} onclose={() => onclose?.()}>
  <div class="grid gap-4">
    {#if errorMsg}
      <div class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">{errorMsg}</div>
    {/if}

    <div class="grid gap-1.5">
      <Label for="export-page-range" class="text-xs text-muted-foreground">Sayfa Aralığı (Tümü için boş bırakın)</Label>
      <Input id="export-page-range" type="text" bind:value={pageRange} placeholder="Örn: 1-5, 8, 11-13" />
      <small class="text-xs text-muted-foreground">Toplam Sayfa: {maxPages}</small>
    </div>

    <div class="grid grid-cols-4 gap-0.5 rounded-md bg-black/5 p-0.5 dark:bg-white/10" role="group" aria-label="Dışa aktarma türü">
      <button type="button" class="rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs" aria-pressed={exportType === 'print'} onclick={() => exportType = 'print'}>Yazdır</button>
      <button type="button" class="rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs" aria-pressed={exportType === 'image'} onclick={() => exportType = 'image'}>Resim (Görsel)</button>
      <button type="button" class="rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs" aria-pressed={exportType === 'text'} onclick={() => exportType = 'text'}>Metin (StExt)</button>
      <button type="button" class="rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs" aria-pressed={exportType === 'document'} onclick={() => exportType = 'document'}>Belge</button>
    </div>

    {#if exportType === 'image'}
      <div class="grid grid-cols-2 gap-3">
        <div class="grid gap-1.5">
          <Label for="export-image-format" class="text-xs text-muted-foreground">Format</Label>
          <select id="export-image-format" bind:value={imgFormat} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
            <option value="png">PNG (Yüksek Kalite)</option>
            <option value="jpeg">JPEG (Daha Küçük Boyut)</option>
          </select>
        </div>
        <div class="grid gap-1.5">
          <Label for="export-image-dpi" class="text-xs text-muted-foreground">Çözünürlük (DPI)</Label>
          <select id="export-image-dpi" bind:value={imgDpi} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
            <option value={150}>150 DPI (Standart Okuma)</option>
            <option value={300}>300 DPI (Baskı Kalitesi)</option>
            <option value={600}>600 DPI (Yüksek Detay)</option>
          </select>
        </div>
      </div>
    {:else if exportType === 'text'}
      <div class="grid gap-1.5">
        <Label for="export-text-format" class="text-xs text-muted-foreground">Metin Formatı</Label>
        <select id="export-text-format" bind:value={txtFormat} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
          <option value="markdown">Markdown (.md)</option>
          <option value="txt">Düz Metin (.txt)</option>
        </select>
      </div>
    {:else if exportType === 'document'}
      <div class="grid gap-1.5">
        <Label for="export-document-format" class="text-xs text-muted-foreground">Belge Formatı</Label>
        <select id="export-document-format" bind:value={docFormat} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
          {#each Object.entries(DOCUMENT_FORMATS) as [value, { label }] (value)}
            <option {value}>{label}</option>
          {/each}
        </select>
        <small class="text-xs text-muted-foreground">Metin, başlıklar, tablolar ve görseller düzenlenebilir olarak aktarılır; sayfa düzeni birebir korunmaz. Taranmış sayfalar yalnız görsel olarak çıkar.</small>
      </div>
    {:else}
      <p class="text-center text-xs text-muted-foreground">Belge doğrudan işletim sisteminin varsayılan yazdırma kuyruğuna gönderilecektir.</p>
    {/if}
  </div>

  {#snippet footer()}
    <Button variant="outline" size="sm" disabled={isExporting} onclick={() => onclose?.()}>İptal</Button>
    <Button size="sm" disabled={isExporting} onclick={handleExport}>
      {isExporting ? 'İşleniyor...' : (exportType === 'print' ? 'Yazdır' : 'Dışa Aktar')}
    </Button>
  {/snippet}
</AppDialog>
