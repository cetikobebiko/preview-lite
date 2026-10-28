<script>
  import AppDialog from "./AppDialog.svelte";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { Button } from "$lib/components/ui/button";
  import { setPageLabels } from "../pdfState.js";
  import { getSessionPageLabels } from "../services/backend/session.js";
  import { isSignatureLossDeclined } from "../services/signatureLossGuard.js";
  import { PAGE_LABEL_STYLES, formatPageLabel, pageLabelRangesError } from "../services/documentStructure.js";

  let { sessionId = "", contentGeneration = 0, pageCount = 0, editable = true, onsaved, onclose } = $props();

  const selectClass = "h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50";

  // Rows show 1-based start pages; `toWire` converts them.
  let rows = $state([]);
  let loadedRows = $state("[]");
  let loading = $state(true);
  let saving = $state(false);
  let error = $state("");

  $effect(() => {
    const requestSession = sessionId;
    const requestGeneration = contentGeneration;
    if (!requestSession) return;
    let cancelled = false;
    loading = true;
    error = "";
    getSessionPageLabels(requestSession, requestGeneration)
      .then((ranges) => {
        if (cancelled) return;
        rows = ranges.map((range) => ({ ...range, startPage: range.startPage + 1 }));
        loadedRows = JSON.stringify(rows);
      })
      .catch((e) => { if (!cancelled) error = `Sayfa etiketleri okunamadı: ${e?.message ?? e}`; })
      .finally(() => { if (!cancelled) loading = false; });
    return () => { cancelled = true; };
  });

  const toWire = (list) => list.map((row) => ({
    startPage: Number(row.startPage) - 1,
    style: row.style,
    prefix: row.prefix,
    firstNumber: Number(row.firstNumber),
  }));
  const wire = $derived(toWire(rows));
  const validationError = $derived(pageLabelRangesError(wire, pageCount));
  const changed = $derived(JSON.stringify(rows) !== loadedRows);
  const canSave = $derived(editable && !loading && !saving && changed && !validationError);

  // An invalid next start (out of range, not after this one) previews this range to the last page.
  function rangeEnd(index) {
    const next = wire[index + 1]?.startPage;
    const last = pageCount - 1;
    return Number.isInteger(next) && next > wire[index].startPage && next <= last ? next - 1 : last;
  }

  function preview(index) {
    const range = wire[index];
    if (!range || pageLabelRangesError(wire.slice(0, index + 1), pageCount)) return "";
    const end = Math.max(range.startPage, rangeEnd(index));
    const first = formatPageLabel(range, range.startPage);
    return end > range.startPage ? `${first} … ${formatPageLabel(range, end)}` : first;
  }

  function addRow() {
    const last = rows[rows.length - 1];
    const startPage = last ? Math.min(pageCount, Number(last.startPage) + 1) : 1;
    rows = [...rows, { startPage, style: "decimal", prefix: "", firstNumber: 1 }];
  }

  function removeRow(index) {
    rows = rows.filter((_, rowIndex) => rowIndex !== index);
  }

  async function save(ranges) {
    saving = true;
    error = "";
    try {
      const snapshot = await setPageLabels(sessionId, contentGeneration, ranges);
      onsaved?.(snapshot);
      onclose?.();
    } catch (e) {
      if (!isSignatureLossDeclined(e)) error = e?.message ?? String(e);
    } finally {
      saving = false;
    }
  }

  function close() {
    if (!saving) onclose?.();
  }
</script>

<AppDialog title="Sayfa Etiketleri" description="Sayfa numaralandırmasını aralıklarla tanımlayın (ör. ön sözler i, ii; gövde 1, 2)." size="lg" onclose={close}>
  <div class="grid gap-3">
    {#if !editable}
      <p class="rounded-md bg-warning px-3 py-2 text-xs text-warning-foreground">Belge salt okunur açıldı; sayfa etiketleri değiştirilemez.</p>
    {/if}
    {#if loading}
      <p class="text-xs text-muted-foreground">Yükleniyor…</p>
    {:else}
      {#if rows.length === 0}
        <p class="text-xs text-muted-foreground">Belgede sayfa etiketi yok; sayfalar fiziksel sırasıyla (1, 2, 3…) gösterilir.</p>
      {/if}
      {#each rows as row, index (index)}
        <div class="grid grid-cols-[5rem_1fr_1fr_5rem_auto] items-end gap-2">
          <div class="grid gap-1">
            <Label for="label-start-{index}" class="text-[11px] text-muted-foreground">Başlangıç</Label>
            <Input id="label-start-{index}" type="number" min="1" max={pageCount} bind:value={row.startPage} disabled={index === 0 || !editable || saving} />
          </div>
          <div class="grid gap-1">
            <Label for="label-style-{index}" class="text-[11px] text-muted-foreground">Numara stili</Label>
            <select id="label-style-{index}" bind:value={row.style} class={selectClass} disabled={!editable || saving}>
              {#each PAGE_LABEL_STYLES as style (style.value)}
                <option value={style.value}>{style.label}</option>
              {/each}
            </select>
          </div>
          <div class="grid gap-1">
            <Label for="label-prefix-{index}" class="text-[11px] text-muted-foreground">Önek</Label>
            <Input id="label-prefix-{index}" type="text" maxlength="256" bind:value={row.prefix} disabled={!editable || saving} />
          </div>
          <div class="grid gap-1">
            <Label for="label-first-{index}" class="text-[11px] text-muted-foreground">İlk numara</Label>
            <Input id="label-first-{index}" type="number" min="1" bind:value={row.firstNumber} disabled={row.style === "none" || !editable || saving} />
          </div>
          <Button variant="ghost" size="sm" onclick={() => removeRow(index)} disabled={(index === 0 && rows.length > 1) || !editable || saving} aria-label="Aralığı sil">Sil</Button>
          <p class="col-span-5 -mt-1 text-[11px] text-muted-foreground">{preview(index)}</p>
        </div>
      {/each}
      <div>
        <Button variant="outline" size="sm" onclick={addRow} disabled={!editable || saving || (rows.length > 0 && Number(rows[rows.length - 1].startPage) >= pageCount)}>Aralık ekle</Button>
      </div>
      {#if validationError && rows.length > 0}
        <p class="text-xs text-destructive">{validationError}</p>
      {/if}
    {/if}
    {#if error}
      <p class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">{error}</p>
    {/if}
  </div>
  {#snippet footer()}
    <Button variant="outline" size="sm" onclick={close} disabled={saving}>Kapat</Button>
    <Button size="sm" onclick={() => save(wire)} disabled={!canSave}>{saving ? "Kaydediliyor…" : "Kaydet"}</Button>
  {/snippet}
</AppDialog>
