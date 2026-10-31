<script>
  import AppDialog from "./AppDialog.svelte";
  import { Button } from "$lib/components/ui/button";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { importXfdf } from "../pdfState.js";
  import { exportSessionXfdf } from "../services/backend/session.js";
  import { isSignatureLossDeclined } from "../services/signatureLossGuard.js";

  let { sessionId = "", contentGeneration = 0, fileName = "", editable = true, onimported, onexported, onclose } = $props();

  let includeComments = $state(true);
  let includeFields = $state(true);
  let busy = $state(false);
  let error = $state("");

  async function runExport() {
    error = "";
    const stem = (fileName || "belge").replace(/\.pdf$/i, "");
    const target = await save({ defaultPath: `${stem}.xfdf`, filters: [{ name: "XFDF", extensions: ["xfdf"] }] });
    if (!target) return;
    busy = true;
    try {
      await exportSessionXfdf(sessionId, contentGeneration, includeComments, includeFields, target);
      onexported?.("XFDF dışa aktarıldı.");
      onclose?.();
    } catch (e) {
      error = e?.message ?? String(e);
    } finally {
      busy = false;
    }
  }

  async function runImport() {
    error = "";
    const selected = await open({ multiple: false, filters: [{ name: "XFDF", extensions: ["xfdf", "xml"] }] });
    const source = Array.isArray(selected) ? selected[0] : selected;
    if (!source) return;
    busy = true;
    try {
      const { snapshot, summary } = await importXfdf(sessionId, contentGeneration, source);
      const parts = [];
      if (summary.annotations) parts.push(`${summary.annotations} açıklama`);
      if (summary.fields) parts.push(`${summary.fields} alan değeri`);
      const skipped = summary.skippedAnnotations ? ` (${summary.skippedAnnotations} desteklenmeyen açıklama atlandı)` : "";
      onimported?.(snapshot, `XFDF içe aktarıldı: ${parts.join(", ")}${skipped}.`);
      onclose?.();
    } catch (e) {
      if (!isSignatureLossDeclined(e)) error = e?.message ?? String(e);
    } finally {
      busy = false;
    }
  }

  function close() {
    if (!busy) onclose?.();
  }
</script>

<AppDialog title="Açıklamalar ve Form Verisi (XFDF)" description="Açıklamaları ve form alanı değerlerini XFDF dosyası olarak alıp verin." onclose={close}>
  <div class="grid gap-4 text-xs">
    <section class="grid gap-2">
      <h3 class="text-sm font-medium">Dışa aktar</h3>
      <label class="flex items-center gap-2"><input type="checkbox" bind:checked={includeComments} disabled={busy} /> Açıklamalar</label>
      <label class="flex items-center gap-2"><input type="checkbox" bind:checked={includeFields} disabled={busy} /> Form alanı değerleri</label>
      <Button variant="outline" size="sm" class="justify-self-start" onclick={runExport} disabled={busy || (!includeComments && !includeFields)}>XFDF olarak kaydet…</Button>
    </section>
    <section class="grid gap-2">
      <h3 class="text-sm font-medium">İçe aktar</h3>
      <p class="text-muted-foreground">Alan değerleri aynı adlı alanlara yazılır; açıklamalar belgeye eklenir. Not, vurgu, altı/üstü çizili, dalgalı, kare, daire ve mürekkep açıklamaları desteklenir; diğerleri atlanır.</p>
      {#if !editable}
        <p class="rounded-md bg-warning px-3 py-2 text-warning-foreground">Belge salt okunur açıldı; içe aktarılamaz.</p>
      {/if}
      <Button variant="outline" size="sm" class="justify-self-start" onclick={runImport} disabled={busy || !editable}>XFDF dosyası seç…</Button>
    </section>
    {#if error}
      <p class="rounded-md bg-destructive/10 px-3 py-2 text-destructive" role="alert">{error}</p>
    {/if}
  </div>
  {#snippet footer()}
    <Button variant="outline" size="sm" onclick={close} disabled={busy}>Kapat</Button>
  {/snippet}
</AppDialog>
