<script>
  import AppDialog from "./AppDialog.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { ask, open, save } from "@tauri-apps/plugin-dialog";
  import {
    exportSessionDocument,
    exportSessionEmbeddedImages,
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
  
  // Image options: render whole pages, or save the images embedded in them as they are.
  let imgSource = $state("pages"); // "pages" | "embedded"
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
  
  // Print layout (print_layout in Rust): "as_is" sends the pages unchanged.
  const PRINT_PAPERS = ["A4", "A3", "A5", "US Letter", "US Legal"];
  const PRINT_MODES = {
    as_is: "Olduğu gibi",
    fit: "Kağıda sığdır",
    actual: "Gerçek boyut",
    shrink: "Büyük sayfaları küçült",
    multiple: "Tabakaya birden çok sayfa",
    booklet: "Kitapçık",
    poster: "Poster",
  };
  let printMode = $state("as_is");
  let printPaper = $state("A4");
  let printOrientation = $state("auto");
  let perSheet = $state(2);
  let sheetOrder = $state("horizontal");
  let sheetBorder = $state(false);
  let bookletSides = $state("both");
  let bookletRightBinding = $state(false);
  let posterPercent = $state(200);
  let posterOverlapMm = $state(10);
  let posterCutMarks = $state(true);

  function printLayout() {
    if (printMode === "as_is") return null;
    const mode = { kind: printMode };
    if (printMode === "multiple") Object.assign(mode, { perSheet: Number(perSheet), order: sheetOrder, border: sheetBorder });
    if (printMode === "booklet") Object.assign(mode, { sides: bookletSides, rightBinding: bookletRightBinding });
    if (printMode === "poster") {
      Object.assign(mode, { percent: Number(posterPercent), overlapPoints: (Number(posterOverlapMm) * 72) / 25.4, cutMarks: posterCutMarks });
    }
    return { paper: printPaper, orientation: printMode === "booklet" ? "auto" : printOrientation, mode };
  }

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
                      imgSource === "embedded"
                        ? exportSessionEmbeddedImages(sessionId, contentGeneration, selectedPages, outputDir, overwrite)
                        : exportSessionPagesAsImages(
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
              await printSessionPdf(sessionId, contentGeneration, ranges, printLayout());
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
      <div class="grid gap-1.5">
        <Label for="export-image-source" class="text-xs text-muted-foreground">Ne aktarılsın?</Label>
        <select id="export-image-source" bind:value={imgSource} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
          <option value="pages">Sayfaların görüntüsü</option>
          <option value="embedded">Sayfalardaki gömülü görseller (PNG, özgün çözünürlük)</option>
        </select>
      </div>
      {#if imgSource === 'pages'}
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
      {:else}
      <p class="text-xs text-muted-foreground">Her görsel belgede saklandığı çözünürlükte bir kez kaydedilir; maske ve kırpma uygulanmaz.</p>
      {/if}
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
      <div class="grid gap-3">
        <div class="grid gap-1.5">
          <Label for="print-mode" class="text-xs text-muted-foreground">Sayfa Düzeni</Label>
          <select id="print-mode" bind:value={printMode} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
            {#each Object.entries(PRINT_MODES) as [value, label] (value)}
              <option {value}>{label}</option>
            {/each}
          </select>
        </div>
        {#if printMode !== "as_is"}
          <div class="grid grid-cols-2 gap-3">
            <div class="grid gap-1.5">
              <Label for="print-paper" class="text-xs text-muted-foreground">Kağıt</Label>
              <select id="print-paper" bind:value={printPaper} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
                {#each PRINT_PAPERS as paper (paper)}<option value={paper}>{paper}</option>{/each}
              </select>
            </div>
            {#if printMode !== "booklet"}
              <div class="grid gap-1.5">
                <Label for="print-orientation" class="text-xs text-muted-foreground">Yön</Label>
                <select id="print-orientation" bind:value={printOrientation} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
                  <option value="auto">Otomatik</option>
                  <option value="portrait">Dikey</option>
                  <option value="landscape">Yatay</option>
                </select>
              </div>
            {/if}
          </div>
        {/if}
        {#if printMode === "multiple"}
          <div class="grid grid-cols-2 gap-3">
            <div class="grid gap-1.5">
              <Label for="print-per-sheet" class="text-xs text-muted-foreground">Tabaka başına sayfa</Label>
              <select id="print-per-sheet" bind:value={perSheet} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
                {#each [2, 4, 6, 9, 16] as count (count)}<option value={count}>{count}</option>{/each}
              </select>
            </div>
            <div class="grid gap-1.5">
              <Label for="print-order" class="text-xs text-muted-foreground">Sayfa sırası</Label>
              <select id="print-order" bind:value={sheetOrder} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
                <option value="horizontal">Yatay</option>
                <option value="horizontal_reversed">Yatay (ters)</option>
                <option value="vertical">Dikey</option>
                <option value="vertical_reversed">Dikey (ters)</option>
                <option value="cut_stack">Kes ve diz</option>
              </select>
            </div>
          </div>
          <label class="flex items-center gap-2 text-xs"><input type="checkbox" bind:checked={sheetBorder} /> Sayfa kenarlığı çiz</label>
        {:else if printMode === "booklet"}
          <div class="grid grid-cols-2 gap-3">
            <div class="grid gap-1.5">
              <Label for="print-booklet-sides" class="text-xs text-muted-foreground">Yüzler</Label>
              <select id="print-booklet-sides" bind:value={bookletSides} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
                <option value="both">İki yüz</option>
                <option value="front">Yalnız ön yüz</option>
                <option value="back">Yalnız arka yüz</option>
              </select>
            </div>
            <div class="grid gap-1.5">
              <Label for="print-booklet-binding" class="text-xs text-muted-foreground">Ciltleme</Label>
              <select id="print-booklet-binding" bind:value={bookletRightBinding} class="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
                <option value={false}>Sol</option>
                <option value={true}>Sağ</option>
              </select>
            </div>
          </div>
          <small class="text-xs text-muted-foreground">İki yüz seçiliyken macOS ve Linux'ta kısa kenardan çift taraflı yazdırma istenir.</small>
        {:else if printMode === "poster"}
          <div class="grid grid-cols-2 gap-3">
            <div class="grid gap-1.5">
              <Label for="print-poster-percent" class="text-xs text-muted-foreground">Ölçek (%)</Label>
              <Input id="print-poster-percent" type="number" min="10" max="1000" bind:value={posterPercent} />
            </div>
            <div class="grid gap-1.5">
              <Label for="print-poster-overlap" class="text-xs text-muted-foreground">Bindirme (mm)</Label>
              <Input id="print-poster-overlap" type="number" min="0" max="50" bind:value={posterOverlapMm} />
            </div>
          </div>
          <label class="flex items-center gap-2 text-xs"><input type="checkbox" bind:checked={posterCutMarks} /> Kesim işaretleri</label>
        {/if}
        <p class="text-center text-xs text-muted-foreground">Belge doğrudan işletim sisteminin varsayılan yazdırma kuyruğuna gönderilecektir.</p>
      </div>
    {/if}
  </div>

  {#snippet footer()}
    <Button variant="outline" size="sm" disabled={isExporting} onclick={() => onclose?.()}>İptal</Button>
    <Button size="sm" disabled={isExporting} onclick={handleExport}>
      {isExporting ? 'İşleniyor...' : (exportType === 'print' ? 'Yazdır' : 'Dışa Aktar')}
    </Button>
  {/snippet}
</AppDialog>
