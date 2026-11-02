<script>
  import AppDialog from "./AppDialog.svelte";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { Button } from "$lib/components/ui/button";
  import { updateDocumentMetadata } from "../pdfState.js";
  import { isSignatureLossDeclined } from "../services/signatureLossGuard.js";

  let {
    isOpen = false,
    metadata = {
      title: "",
      author: "",
      subject: "",
      keywords: "",
      creator: "",
    },
    sessionId = "",
    contentGeneration = 0,
    editable = true,
    onsaved,
    onclose,
  } = $props();

  const FIELDS = [
    { key: "title", label: "Belge Başlığı (Title)" },
    { key: "author", label: "Yazar (Author)" },
    { key: "subject", label: "Konu (Subject)" },
    { key: "keywords", label: "Anahtar Kelimeler (Keywords)" },
  ];

  let draft = $state({ title: "", author: "", subject: "", keywords: "" });
  let saving = $state(false);
  let error = $state("");

  // Each opening starts from the document's current values.
  $effect(() => {
    if (!isOpen) return;
    draft = {
      title: metadata.title ?? "",
      author: metadata.author ?? "",
      subject: metadata.subject ?? "",
      keywords: metadata.keywords ?? "",
    };
    error = "";
  });

  const changed = $derived(FIELDS.some(({ key }) => draft[key].trim() !== (metadata[key] ?? "").trim()));
  const canSave = $derived(editable && Boolean(sessionId) && changed && !saving);

  function close() {
    if (!saving) onclose?.();
  }

  async function handleSave() {
    if (!canSave) return;
    saving = true;
    error = "";
    const edit = Object.fromEntries(FIELDS.map(({ key }) => [key, draft[key].trim()]));
    try {
      const snapshot = await updateDocumentMetadata(sessionId, contentGeneration, edit);
      onsaved?.(snapshot, { ...edit, creator: metadata.creator ?? "" });
      onclose?.();
    } catch (e) {
      if (!isSignatureLossDeclined(e)) error = e?.message ?? String(e);
    } finally {
      saving = false;
    }
  }
</script>

{#if isOpen}
  <AppDialog title="PDF Meta Verileri" description="Başlık, yazar, konu ve anahtar kelimeler belgeye artımlı olarak yazılır." onclose={close}>
    <div class="grid gap-3">
      {#if !editable}
        <p class="rounded-md bg-warning px-3 py-2 text-xs text-warning-foreground">Belge salt okunur açıldı; meta veri değiştirilemez.</p>
      {/if}
      {#each FIELDS as field (field.key)}
        <div class="grid gap-1.5">
          <Label for="meta-{field.key}" class="text-xs text-muted-foreground">{field.label}</Label>
          <Input id="meta-{field.key}" type="text" bind:value={draft[field.key]} disabled={!editable || saving} />
        </div>
      {/each}
      <div class="grid gap-1.5">
        <Label for="meta-creator" class="text-xs text-muted-foreground">Oluşturan Uygulama (Creator)</Label>
        <Input id="meta-creator" type="text" value={metadata.creator} readonly disabled />
      </div>
      {#if error}
        <p class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">{error}</p>
      {/if}
    </div>
    {#snippet footer()}
      <Button variant="outline" size="sm" onclick={close} disabled={saving}>Kapat</Button>
      <Button size="sm" onclick={handleSave} disabled={!canSave}>{saving ? "Kaydediliyor…" : "Kaydet"}</Button>
    {/snippet}
  </AppDialog>
{/if}
