<script>
  import { onDestroy, onMount, untrack } from "svelte";
  import AppDialog from "./AppDialog.svelte";
  import OptimizeSettingsFields from "./OptimizeSettingsFields.svelte";
  import SecuritySettingsFields from "./SecuritySettingsFields.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    BATCH_OPERATIONS,
    CONFLICT_POLICIES,
    DOCUMENT_FORMATS,
    IMAGE_FORMATS,
    TEXT_FORMATS,
    batchActions,
    batchReportText,
    buildBatchOperation,
    createBatchTracker,
    fileStateLabel,
  } from "../services/app/batchController.js";
  import { optimizeSettingsError, presetSettings } from "../services/pdfOptimizer.js";
  import { emptySecurityFields, securityFieldsError, securityOpFromFields } from "../services/documentSecurity.js";

  let { onclose, onToast = () => {}, createTracker = createBatchTracker } = $props();

  const MAX_FILES = 500;
  const MIN_DPI = 36;
  const MAX_DPI = 1200;
  const STEPS = Object.freeze(["Dosyalar", "İşlem", "Çıktı", "Çalıştırma", "Rapor"]);
  const selectClass = "h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50";
  const REPORT_FILTERS = Object.freeze([
    { value: "all", label: "Tümü" },
    { value: "completed", label: "Tamamlanan" },
    { value: "skipped", label: "Atlanan" },
    { value: "failed", label: "Hatalı" },
    { value: "cancelled", label: "İptal" },
  ]);

  let step = $state(0);
  let loading = $state(true);
  let error = $state("");

  // Step 1
  let sources = $state([]);
  let recursive = $state(false);
  let scanning = $state(false);
  let scan = $state({ files: [], rejected: [], truncated: false });
  let excluded = $state([]);

  // Step 2
  let operation = $state("optimize");
  let optimizePreset = $state("standard");
  let optimizeSettings = $state(presetSettings("standard"));
  let documentFormat = $state("docx");
  let textFormat = $state("txt");
  let imageFormat = $state("png");
  let dpi = $state(144);
  let securityFields = $state(emptySecurityFields());

  // Step 3
  let outputDir = $state("");
  let conflictPolicy = $state("number");

  // Steps 4–5
  let status = $state(null);
  let starting = $state(false);
  let cancelling = $state(false);
  let reportFilter = $state("all");

  // One tracker for the dialog's lifetime; a later prop change does not replace it.
  const tracker = untrack(() => createTracker({ onChange: (next) => { status = next; } }));

  const inputs = $derived(scan.files.filter((file) => !excluded.includes(file.path)));
  const filesError = $derived(
    inputs.length === 0 ? "En az bir PDF seçin."
      : inputs.length > MAX_FILES ? `Bir toplu işlem en fazla ${MAX_FILES} dosya alır.` : null,
  );
  const operationError = $derived.by(() => {
    if (operation === "optimize") return optimizeSettingsError(optimizeSettings);
    if (operation === "security") return securityFieldsError(securityFields);
    if (operation === "export_images") {
      const value = Number(dpi);
      if (!Number.isFinite(value) || value < MIN_DPI || value > MAX_DPI) {
        return `Çözünürlük ${MIN_DPI}–${MAX_DPI} DPI aralığında olmalı.`;
      }
    }
    return null;
  });
  const outputError = $derived(outputDir ? null : "Bir çıktı klasörü seçin.");
  const running = $derived(status?.state === "running");
  const finishedCount = $derived(status ? status.counts.completed + status.counts.skipped + status.counts.failed + status.counts.cancelled : 0);
  const currentFile = $derived(status?.files.find((file) => file.state === "running") ?? null);
  const operationLabel = $derived(BATCH_OPERATIONS.find((entry) => entry.id === (status?.operation ?? operation))?.label ?? "");
  const reportFiles = $derived(
    status ? status.files.filter((file) => reportFilter === "all" || file.state === reportFilter) : [],
  );

  // A finished batch moves the wizard to its report.
  $effect(() => {
    if (status && status.state !== "running" && step === 3) step = 4;
  });

  onMount(async () => {
    try {
      const existing = await tracker.attach(null);
      if (existing) step = existing.state === "running" ? 3 : 4;
    } catch (e) {
      error = e?.message ?? String(e);
    } finally {
      loading = false;
    }
  });

  onDestroy(() => tracker.dispose());

  async function rescan() {
    if (sources.length === 0) {
      scan = { files: [], rejected: [], truncated: false };
      return;
    }
    scanning = true;
    error = "";
    try {
      scan = await batchActions.scan($state.snapshot(sources), recursive);
    } catch (e) {
      error = e?.message ?? String(e);
    } finally {
      scanning = false;
    }
  }

  async function addFiles() {
    const selected = await open({ multiple: true, filters: [{ name: "PDF", extensions: ["pdf"] }] });
    if (!selected) return;
    const picked = Array.isArray(selected) ? selected : [selected];
    sources = [...new Set([...sources, ...picked])];
    await rescan();
  }

  async function addFolder() {
    const selected = await open({ directory: true, multiple: false });
    if (!selected) return;
    sources = [...new Set([...sources, selected])];
    await rescan();
  }

  async function toggleRecursive(event) {
    recursive = event.currentTarget.checked;
    await rescan();
  }

  function removeFile(path) {
    excluded = [...excluded, path];
  }

  function clearFiles() {
    sources = [];
    excluded = [];
    scan = { files: [], rejected: [], truncated: false };
  }

  async function chooseOutputDir() {
    const selected = await open({ directory: true, multiple: false });
    if (selected) outputDir = selected;
  }

  async function start() {
    error = "";
    starting = true;
    try {
      const request = {
        inputs: inputs.map((file) => file.path),
        operation: buildBatchOperation(operation, {
          optimizeSettings: $state.snapshot(optimizeSettings),
          documentFormat,
          textFormat,
          imageFormat,
          dpi,
          securityOp: securityOpFromFields($state.snapshot(securityFields)),
        }),
        outputDir,
        conflictPolicy,
      };
      const { batchId } = await batchActions.start(request);
      // Passwords are not kept once the backend has the request.
      securityFields = emptySecurityFields();
      step = 3;
      await tracker.attach(batchId);
    } catch (e) {
      error = e?.message ?? String(e);
    } finally {
      starting = false;
    }
  }

  async function cancel() {
    if (!status) return;
    cancelling = true;
    try {
      await batchActions.cancel(status.batchId);
    } catch (e) {
      error = e?.message ?? String(e);
    } finally {
      cancelling = false;
    }
  }

  async function revealOutput() {
    try {
      await batchActions.reveal(status.batchId);
    } catch (e) {
      error = e?.message ?? String(e);
    }
  }

  async function copyReport() {
    try {
      await navigator.clipboard.writeText(batchReportText(status, operationLabel));
      onToast("Toplu işlem raporu panoya kopyalandı.");
    } catch (e) {
      error = `Rapor panoya kopyalanamadı: ${e?.message ?? String(e)}`;
    }
  }

  function newBatch() {
    status = null;
    error = "";
    clearFiles();
    step = 0;
  }

  function formatSize(bytes) {
    if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  }

  function baseName(path) {
    return path.split(/[\\/]/).pop() || path;
  }
</script>

<AppDialog
  title="Toplu İşlem"
  description="Seçtiğiniz PDF'lere aynı işlemi uygular ve sonuçları çıktı klasörüne yazar. Girdi dosyaları ve açık belge değişmez."
  size="lg"
  onclose={() => onclose?.()}
>
  <div class="grid gap-3" data-testid="batch-wizard">
    <ol class="flex flex-wrap gap-1 text-[11px]" aria-label="Adımlar">
      {#each STEPS as label, index (label)}
        <li class="rounded px-2 py-0.5 {index === step ? 'bg-primary text-primary-foreground' : 'bg-black/5 text-muted-foreground dark:bg-white/10'}" aria-current={index === step ? "step" : undefined}>{index + 1}. {label}</li>
      {/each}
    </ol>

    {#if loading}
      <p class="text-xs text-muted-foreground">Yükleniyor…</p>
    {:else if step === 0}
      <div class="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="outline" onclick={addFiles} disabled={scanning}>Dosya ekle…</Button>
        <Button size="sm" variant="outline" onclick={addFolder} disabled={scanning}>Klasör ekle…</Button>
        <label class="flex items-center gap-2 text-xs"><input type="checkbox" checked={recursive} onchange={toggleRecursive} disabled={scanning} /> Alt klasörleri dahil et</label>
        {#if sources.length > 0}<Button size="sm" variant="ghost" onclick={clearFiles} disabled={scanning}>Listeyi temizle</Button>{/if}
      </div>
      {#if scanning}<p class="text-xs text-muted-foreground" role="status">Taranıyor…</p>{/if}
      <p class="text-xs text-muted-foreground">{inputs.length} PDF seçildi (en fazla {MAX_FILES}).</p>
      {#if scan.truncated}
        <p class="text-xs text-destructive">Seçimde {MAX_FILES} dosyadan fazlası var; yalnız ilk {MAX_FILES} dosya listelendi.</p>
      {/if}
      {#if inputs.length > 0}
        <ul class="max-h-56 overflow-auto rounded-md border border-border text-xs" aria-label="Seçilen dosyalar">
          {#each inputs as file (file.path)}
            <li class="flex items-center justify-between gap-2 border-b border-border px-2 py-1 last:border-0">
              <span class="truncate" title={file.path}>{baseName(file.path)}</span>
              <span class="flex shrink-0 items-center gap-2 text-muted-foreground">{formatSize(file.size)}
                <button type="button" class="text-destructive" aria-label="{baseName(file.path)} dosyasını çıkar" onclick={() => removeFile(file.path)}>Çıkar</button>
              </span>
            </li>
          {/each}
        </ul>
      {/if}
      {#if scan.rejected.length > 0}
        <details class="text-xs">
          <summary>{scan.rejected.length} dosya alınmadı</summary>
          <ul class="mt-1 grid gap-0.5" aria-label="Alınmayan dosyalar">
            {#each scan.rejected as rejected (rejected.path)}
              <li><span title={rejected.path}>{baseName(rejected.path)}</span>: {rejected.message}</li>
            {/each}
          </ul>
        </details>
      {/if}
      <p class="text-[11px] text-muted-foreground">Sembolik bağlar, hard link'ler ve uygulamanın özel alanındaki dosyalar alınmaz. Parola isteyen dosyalar atlanır; toplu işlemde parola sorulmaz.</p>
    {:else if step === 1}
      <div class="grid gap-1">
        <Label for="batch-operation" class="text-[11px] text-muted-foreground">İşlem</Label>
        <select id="batch-operation" bind:value={operation} class={selectClass}>
          {#each BATCH_OPERATIONS as entry (entry.id)}
            <option value={entry.id}>{entry.label}</option>
          {/each}
        </select>
      </div>
      {#if operation === "optimize"}
        <OptimizeSettingsFields bind:settings={optimizeSettings} bind:presetId={optimizePreset} idPrefix="batch-optimize" />
        <p class="text-[11px] text-muted-foreground">Yalnız sahip parolasıyla kısıtlanmış belgeler, kopya kısıtlamaları kaldıracağı için atlanır.</p>
      {:else if operation === "export_document"}
        <div class="grid gap-1">
          <Label for="batch-document-format" class="text-[11px] text-muted-foreground">Biçim</Label>
          <select id="batch-document-format" bind:value={documentFormat} class={selectClass}>
            {#each DOCUMENT_FORMATS as format (format.value)}<option value={format.value}>{format.label}</option>{/each}
          </select>
        </div>
      {:else if operation === "export_text"}
        <div class="grid gap-1">
          <Label for="batch-text-format" class="text-[11px] text-muted-foreground">Biçim</Label>
          <select id="batch-text-format" bind:value={textFormat} class={selectClass}>
            {#each TEXT_FORMATS as format (format.value)}<option value={format.value}>{format.label}</option>{/each}
          </select>
        </div>
      {:else if operation === "export_images"}
        <div class="grid grid-cols-2 gap-2">
          <div class="grid gap-1">
            <Label for="batch-image-format" class="text-[11px] text-muted-foreground">Biçim</Label>
            <select id="batch-image-format" bind:value={imageFormat} class={selectClass}>
              {#each IMAGE_FORMATS as format (format.value)}<option value={format.value}>{format.label}</option>{/each}
            </select>
          </div>
          <div class="grid gap-1">
            <Label for="batch-image-dpi" class="text-[11px] text-muted-foreground">Çözünürlük (DPI)</Label>
            <Input id="batch-image-dpi" type="number" min={MIN_DPI} max={MAX_DPI} bind:value={dpi} />
          </div>
        </div>
        <p class="text-[11px] text-muted-foreground">Her sayfa ayrı bir dosya olur: rapor_sayfa_1.png, rapor_sayfa_2.png…</p>
      {:else if operation === "security"}
        <SecuritySettingsFields
          bind:fields={securityFields}
          idPrefix="batch-security"
          removeNote="Her dosyanın şifresiz bir kopyası yazılır. Parola isteyen ya da yalnız sahip parolasıyla kısıtlanmış dosyalar atlanır."
        />
        {#if securityFields.mode === "protect"}
          <p class="text-[11px] text-muted-foreground">Aynı parolalar tüm kopyalara uygulanır. Parolalar rapora ya da günlüğe yazılmaz. Yalnız sahip parolasıyla kısıtlanmış dosyalar atlanır.</p>
        {/if}
      {/if}
      {#if operationError}<p class="text-xs text-destructive">{operationError}</p>{/if}
    {:else if step === 2}
      <div class="grid gap-1">
        <span class="text-[11px] text-muted-foreground">Çıktı klasörü</span>
        <div class="flex items-center gap-2">
          <Button size="sm" variant="outline" onclick={chooseOutputDir}>Klasör seç…</Button>
          <span class="truncate text-xs" title={outputDir}>{outputDir || "Seçilmedi"}</span>
        </div>
      </div>
      <div class="grid gap-1">
        <Label for="batch-conflict" class="text-[11px] text-muted-foreground">Aynı adlı dosya varsa</Label>
        <select id="batch-conflict" bind:value={conflictPolicy} class={selectClass}>
          {#each CONFLICT_POLICIES as policy (policy.value)}<option value={policy.value}>{policy.label}</option>{/each}
        </select>
      </div>
      {#if conflictPolicy === "overwrite"}
        <p class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">Çıktı klasöründeki aynı adlı dosyalar değiştirilir. Girdi dosyalarının ve bu toplu işlemin kendi çıktılarının üzerine hiçbir zaman yazılmaz; onlar numaralandırılır.</p>
      {/if}
      <p class="text-[11px] text-muted-foreground">Çıktı adları: rapor_küçültülmüş.pdf, rapor_korumalı.pdf, rapor.docx, rapor.txt, rapor_sayfa_3.png. Her dosyanın çıktısı ayrı ve atomik yazılır.</p>
      {#if outputError}<p class="text-xs text-destructive">{outputError}</p>{/if}
    {:else if step === 3 && status}
      <div class="grid gap-1" role="status" aria-live="polite">
        <p class="text-xs font-medium">{operationLabel}: {finishedCount} / {status.total}</p>
        <progress class="w-full" max={status.total} value={finishedCount}></progress>
        {#if currentFile}<p class="truncate text-xs text-muted-foreground" title={currentFile.inputPath}>İşleniyor: {baseName(currentFile.inputPath)}</p>{/if}
        {#if !running}<p class="text-xs text-muted-foreground">Toplu işlem bitti.</p>{/if}
      </div>
      <ul class="max-h-56 overflow-auto rounded-md border border-border text-xs" aria-label="Dosya ilerlemesi">
        {#each status.files as file (file.index)}
          <li class="flex items-center justify-between gap-2 border-b border-border px-2 py-1 last:border-0">
            <span class="truncate" title={file.inputPath}>{baseName(file.inputPath)}</span>
            <span class="shrink-0 text-muted-foreground">{fileStateLabel(file.state)}</span>
          </li>
        {/each}
      </ul>
      <p class="text-[11px] text-muted-foreground">Pencereyi kapatabilirsiniz; toplu işlem arka planda sürer. Toplu İşlem'i yeniden açınca ilerleme kaldığı yerden görünür.</p>
    {:else if step === 4 && status}
      <div class="grid gap-1 rounded-md bg-black/5 px-3 py-2 text-xs dark:bg-white/10" role="status">
        <p class="font-medium">
          {status.state === "cancelled" ? "Toplu işlem iptal edildi" : status.state === "failed" ? "Toplu işlem durdu" : "Toplu işlem tamamlandı"}
        </p>
        <p>Tamamlanan: {status.counts.completed} · Atlanan: {status.counts.skipped} · Hatalı: {status.counts.failed} · İptal: {status.counts.cancelled}</p>
        {#if status.reason}<p class="text-destructive">{status.reason.message}</p>{/if}
        {#each status.warnings as warning, index (index)}<p class="text-muted-foreground">{warning}</p>{/each}
      </div>
      <div class="flex flex-wrap gap-1" role="group" aria-label="Rapor filtresi">
        {#each REPORT_FILTERS as filter (filter.value)}
          <Button size="sm" variant={reportFilter === filter.value ? "default" : "outline"} aria-pressed={reportFilter === filter.value} onclick={() => (reportFilter = filter.value)}>{filter.label}</Button>
        {/each}
      </div>
      <ul class="max-h-64 overflow-auto rounded-md border border-border text-xs" aria-label="Sonuç raporu">
        {#each reportFiles as file (file.index)}
          <li class="grid gap-0.5 border-b border-border px-2 py-1 last:border-0">
            <span class="flex justify-between gap-2"><span class="truncate" title={file.inputPath}>{baseName(file.inputPath)}</span><span class="shrink-0">{fileStateLabel(file.state)}</span></span>
            {#if file.reason}<span class="text-muted-foreground">{file.reason.message}</span>{/if}
            {#each file.outputs as output (output)}<span class="truncate text-muted-foreground" title={output}>→ {baseName(output)}</span>{/each}
          </li>
        {:else}
          <li class="px-2 py-1 text-muted-foreground">Bu filtrede dosya yok.</li>
        {/each}
      </ul>
    {/if}

    {#if error}
      <p class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">{error}</p>
    {/if}
  </div>
  {#snippet footer()}
    {#if step <= 2}
      <Button variant="outline" size="sm" onclick={() => onclose?.()}>Kapat</Button>
      {#if step > 0}<Button variant="outline" size="sm" onclick={() => (step -= 1)} disabled={starting}>Geri</Button>{/if}
      {#if step === 0}<Button size="sm" onclick={() => (step = 1)} disabled={Boolean(filesError) || scanning}>İleri</Button>{/if}
      {#if step === 1}<Button size="sm" onclick={() => (step = 2)} disabled={Boolean(operationError)}>İleri</Button>{/if}
      {#if step === 2}<Button size="sm" onclick={start} disabled={Boolean(outputError || filesError || operationError) || starting}>{starting ? "Başlatılıyor…" : "Başlat"}</Button>{/if}
    {:else if step === 3}
      <Button variant="outline" size="sm" onclick={() => onclose?.()}>Arka planda sürdür</Button>
      <Button variant="destructive" size="sm" onclick={cancel} disabled={!running || cancelling}>{cancelling ? "İptal ediliyor…" : "İptal et"}</Button>
    {:else}
      <Button variant="outline" size="sm" onclick={revealOutput}>Çıktı klasörünü aç</Button>
      <Button variant="outline" size="sm" onclick={copyReport}>Raporu kopyala</Button>
      <Button variant="outline" size="sm" onclick={newBatch}>Yeni toplu işlem</Button>
      <Button size="sm" onclick={() => onclose?.()}>Kapat</Button>
    {/if}
  {/snippet}
</AppDialog>
