<script>
  import AppDialog from "./AppDialog.svelte";
  let {
    isOpen = false,
    fileName = "",
    totalPages = 1,
    currentPage = 1,
    readingTimeSeconds = 0,
    scale = 1.0,
    onclose,
  } = $props();

  function close() {
    onclose?.();
  }

  function formatTime(totalSecs) {
    const mins = Math.floor(totalSecs / 60);
    const secs = totalSecs % 60;
    if (mins > 0) {
      return `${mins} dk ${secs} sn`;
    }
    return `${secs} sn`;
  }

  const progressPercent = $derived(Math.round((currentPage / Math.max(1, totalPages)) * 100));
  const avgTimePerPage = $derived(totalPages > 0 ? Math.round(readingTimeSeconds / Math.max(1, currentPage)) : 0);
</script>

{#if isOpen}
  <AppDialog title="Oturum ve Okuma İstatistikleri" description={fileName} onclose={close}>
    <div class="grid gap-3">
      <div class="rounded-lg border bg-card p-3">
        <div class="mb-2 flex items-center justify-between text-xs text-muted-foreground">
          <span>Okuma İlerlemesi</span>
          <span class="font-semibold text-foreground tabular-nums">{progressPercent}%</span>
        </div>
        <div class="h-1.5 overflow-hidden rounded-full bg-muted">
          <div class="h-full rounded-full bg-primary" style="width: {progressPercent}%;"></div>
        </div>
        <span class="mt-2 block text-xs text-muted-foreground tabular-nums">Sayfa {currentPage} / {totalPages}</span>
      </div>

      <div class="grid grid-cols-2 gap-3">
        {#each [
          ["Toplam Okuma Süresi", formatTime(readingTimeSeconds)],
          ["Sayfa Başına Ortalama Süre", formatTime(avgTimePerPage)],
          ["Hatırlanan Ölçek (Zoom)", `%${Math.round(scale * 100)}`],
          ["Konum Hafızası", "Kayıtlı"],
        ] as [label, value]}
          <div class="flex flex-col gap-1 rounded-lg border bg-card p-3">
            <span class="text-xs text-muted-foreground">{label}</span>
            <span class="text-lg font-semibold tabular-nums">{value}</span>
          </div>
        {/each}
      </div>
    </div>
  </AppDialog>
{/if}
