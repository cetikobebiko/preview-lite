<script>
  import AppDialog from "./AppDialog.svelte";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { Button } from "$lib/components/ui/button";

  let {
    isOpen = false,
    metadata = {
      title: "",
      author: "",
      subject: "",
      keywords: "",
      creator: "",
    },
    onclose,
  } = $props();

  function close() {
    onclose?.();
  }

  function handleSave() {
    // Disabled / unsupported in PDFium
    close();
  }
</script>

{#if isOpen}
  <AppDialog title="PDF Meta Verileri (Salt Okunur)" description="PDFium motoru metadata yazma C API'si sunmamaktadır" onclose={close}>
    <div class="grid gap-3">
      <p class="rounded-md bg-warning px-3 py-2 text-xs text-warning-foreground">PDFium C API metin/alt-text yazma desteği sunmamaktadır; metadata düzenleme desteklenmiyor.</p>
    <div class="grid gap-1.5">
      <Label for="meta-title" class="text-xs text-muted-foreground">Belge Başlığı (Title)</Label>
      <Input id="meta-title" type="text" value={metadata.title} readonly disabled />
    </div>
    <div class="grid gap-1.5">
      <Label for="meta-author" class="text-xs text-muted-foreground">Yazar (Author)</Label>
      <Input id="meta-author" type="text" value={metadata.author} readonly disabled />
    </div>
    <div class="grid gap-1.5">
      <Label for="meta-subject" class="text-xs text-muted-foreground">Konu (Subject)</Label>
      <Input id="meta-subject" type="text" value={metadata.subject} readonly disabled />
    </div>
    <div class="grid gap-1.5">
      <Label for="meta-keywords" class="text-xs text-muted-foreground">Anahtar Kelimeler (Keywords)</Label>
      <Input id="meta-keywords" type="text" value={metadata.keywords} readonly disabled />
    </div>
    <div class="grid gap-1.5">
      <Label for="meta-creator" class="text-xs text-muted-foreground">Oluşturan Uygulama (Creator)</Label>
      <Input id="meta-creator" type="text" value={metadata.creator} readonly disabled />
    </div>
    </div>
    {#snippet footer()}
      <Button variant="outline" size="sm" onclick={close}>Kapat</Button>
      <Button size="sm" disabled title="PDFium C API metin/alt-text yazma desteği sunmamaktadır">Kaydet (Desteklenmiyor)</Button>
    {/snippet}
  </AppDialog>
{/if}
