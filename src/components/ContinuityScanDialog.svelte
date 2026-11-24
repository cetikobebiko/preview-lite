<script>
  import SmartphoneIcon from "@lucide/svelte/icons/smartphone";
  import AppDialog from "./AppDialog.svelte";
  import { Button } from "$lib/components/ui/button";

  // Continuity Camera taraması sürerken gösterilir. Menü ve kamera iPhone/iPad ile
  // sistem tarafında açılır; burada yalnız bekleme durumu ve iptal vardır. Escape ve
  // kapatma düğmesi de iptal eder. Kapanmayı (sonuç, hata, iptal) controller yönetir.
  let { oncancel } = $props();

  let cancelling = $state(false);

  function cancel() {
    if (cancelling) return;
    cancelling = true;
    oncancel?.();
  }
</script>

<AppDialog title="iPhone/iPad'den bekleniyor…" size="sm" outsideClose={false} onclose={cancel}>
  <div class="flex items-start gap-3" role="status" aria-live="polite">
    <SmartphoneIcon class="mt-0.5 size-5 shrink-0 text-primary" strokeWidth={1.75} />
    <div class="grid gap-2">
      <p>
        Açılan menüden <strong>Fotoğraf Çek</strong>, <strong>Belgeleri Tara</strong> ya da
        <strong>Çizim Ekle</strong>'yi seçin ve iPhone veya iPad'de işlemi tamamlayın.
      </p>
      <p class="text-muted-foreground">
        Gelen sayfalar tek adımda eklenir ve ⌘Z ile geri alınabilir. Cihaz aynı Apple hesabıyla
        oturum açmış, Wi-Fi ve Bluetooth açık ve Mac'e yakın olmalıdır.
      </p>
      {#if cancelling}
        <p class="text-muted-foreground">İptal ediliyor…</p>
      {/if}
    </div>
  </div>
  {#snippet footer()}
    <Button variant="outline" size="sm" disabled={cancelling} onclick={cancel}>İptal</Button>
  {/snippet}
</AppDialog>
