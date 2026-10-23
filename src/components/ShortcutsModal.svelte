<script>
  import AppDialog from "./AppDialog.svelte";

  let { onclose } = $props();

  const shortcuts = [
    { key: "Ctrl + O", desc: "Dosya Aç" },
    { key: "Ctrl + S", desc: "Kaydet (Burn & Save)" },
    { key: "Ctrl + P", desc: "Yazdır" },
    { key: "Ctrl + F", desc: "Arama Çubuğunu Aç/Kapat" },
    { key: "Ctrl + G", desc: "Sayfaya Git" },
    { key: "Ctrl + Z", desc: "Son Değişikliği Geri Al" },
    { key: "Ctrl + Shift + Z / Ctrl + Y", desc: "Geri Alınanı Yinele" },
    { key: "Ok Tuşları", desc: "Sayfalar Arası Geçiş" },
    { key: "PageUp / PageDown", desc: "Sayfalar Arası Geçiş" },
    { key: "Ctrl + (+/-)", desc: "Yakınlaştır / Uzaklaştır" },
    { key: "Ctrl + 0", desc: "Yakınlaştırmayı Sıfırla" },
    { key: "Ctrl + / veya ?", desc: "Kısayol Menüsünü Aç/Kapat" }
  ];
</script>

<AppDialog title="Klavye Kısayolları" size="sm" {onclose}>
  <dl class="divide-y">
    {#each shortcuts as sc}
      <div class="flex items-center justify-between gap-4 py-2">
        <dd class="text-foreground">{sc.desc}</dd>
        <dt><kbd class="rounded-md border bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground">{sc.key}</kbd></dt>
      </div>
    {/each}
  </dl>
</AppDialog>
