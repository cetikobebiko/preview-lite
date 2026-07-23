<script>
  import { onDestroy, createEventDispatcher, tick } from "svelte";

  export let isOpen = false;
  const dispatch = createEventDispatcher();

  let searchQuery = "";
  let selectedIndex = 0;
  let inputRef;

  const commands = [
    { id: "open-metadata", title: "Belge Meta Verilerini Düzenle (Title, Author, Keywords)", icon: "🏷️", category: "Güvenlik & Meta" },
    { id: "export-page-image", title: "Aktif Sayfayı Yüksek Çözünürlüklü SVG / Görsel Olarak Aktar", icon: "🎨", category: "Dışa Aktar" },
    { id: "open-bionic", title: "Biyonik Okuma Katmanı (Bionic Reading)", icon: "🧠", category: "Okuma Modları" },
    { id: "toggle-ruler", title: "Odak Cetvelini Aç / Gizle (Line Focus)", icon: "📏", category: "Okuma Modları" },
    { id: "open-analytics", title: "Oturum ve Okuma İstatistikleri", icon: "📊", category: "İstatistik" },
    { id: "camera-scan", title: "iPhone Süreklilik Kamerası ile Belge Tara & Ekle", icon: "📱", category: "Donanım" },
    { id: "speak-page", title: "Sayfayı Sesli Oku (macOS TTS - Türkçe)", icon: "🔊", category: "Ses" },
    { id: "stop-speak", title: "Sesli Okumayı Durdur", icon: "⏹️", category: "Ses" },
    { id: "share-macos", title: "macOS Paylaşım Menüsü (Share Sheet)", icon: "📤", category: "Paylaşım" },
    { id: "open-ai-assistant", title: "Yapay Zeka Asistanı / Belge Özetle (Local AI)", icon: "🤖", category: "Yapay Zeka" },
    { id: "run-ocr-page", title: "Taranmış Sayfaya OCR Uygula (Apple Vision)", icon: "👁️", category: "Yerel OCR" },
    { id: "toggle-dark-mode", title: "Akıllı Gece Modu (Smart Dark Mode)", icon: "🌙", category: "Görünüm" },
    { id: "toggle-book-mode", title: "Kitap Modu (Çift Sayfa)", icon: "📖", category: "Görünüm" },
    { id: "toggle-split-view", title: "Çift Görünüm / Split View", icon: "📑", category: "Görünüm" },
    { id: "toggle-sidebar", title: "Sol Paneli Göster / Gizle", icon: "🖼️", category: "Arayüz" },
    { id: "open-search", title: "Belge İçi Metin Arama (Cmd+F)", icon: "🔍", category: "Gezinti" },
    { id: "export-notes-md", title: "Notları Markdown (.md) Olarak Dışa Aktar", icon: "📝", category: "Not Alma" },
    { id: "split-pdf", title: "PDF Sayfalarını Böl / Seçili Sayfaları Dışa Aktar", icon: "✂️", category: "Düzenleme" },
    { id: "merge-pdf", title: "Başka Bir PDF ile Birleştir", icon: "➕", category: "Düzenleme" },
    { id: "reset-zoom", title: "Yakınlaştırmayı Sıfırla (%100)", icon: "🎯", category: "Görünüm" },
    { id: "burn-in-pdf", title: "Çizimleri PDF Üzerine Kalıcı İşle & Kaydet (Flattening)", icon: "💾", category: "Dışa Aktar" },
  ];

  $: filteredCommands = commands.filter((cmd) =>
    cmd.title.toLowerCase().includes(searchQuery.toLowerCase()) ||
    cmd.category.toLowerCase().includes(searchQuery.toLowerCase())
  );

  $: if (isOpen) {
    searchQuery = "";
    selectedIndex = 0;
    tick().then(() => {
      if (inputRef) inputRef.focus();
    });
  }

  function handleKeyDown(e) {
    if (!isOpen) return;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      selectedIndex = (selectedIndex + 1) % Math.max(1, filteredCommands.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selectedIndex = (selectedIndex - 1 + filteredCommands.length) % Math.max(1, filteredCommands.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (filteredCommands[selectedIndex]) {
        executeCommand(filteredCommands[selectedIndex].id);
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }

  function executeCommand(id) {
    dispatch("select", id);
    close();
  }

  function close() {
    isOpen = false;
    dispatch("close");
  }
</script>

<svelte:window on:keydown={handleKeyDown} />

{#if isOpen}
  <!-- Overlay Backdrop -->
  <div
    class="cmd-overlay"
    on:click={close}
    on:keydown={(e) => e.key === 'Escape' && close()}
    role="presentation"
    tabindex="-1"
  >
    <div class="cmd-dialog" on:click|stopPropagation role="presentation">
      <div class="cmd-header">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="11" cy="11" r="8"></circle>
          <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
        </svg>
        <input
          bind:this={inputRef}
          type="text"
          class="cmd-input"
          placeholder="Komut veya işlem arayın... (Cmd + K)"
          bind:value={searchQuery}
        />
        <span class="cmd-badge">ESC</span>
      </div>

      <div class="cmd-list">
        {#each filteredCommands as cmd, index}
          <button
            class="cmd-item {index === selectedIndex ? 'active' : ''}"
            on:click={() => executeCommand(cmd.id)}
            on:mouseenter={() => (selectedIndex = index)}
          >
            <span class="cmd-icon">{cmd.icon}</span>
            <div class="cmd-text">
              <span class="cmd-title">{cmd.title}</span>
              <span class="cmd-category">{cmd.category}</span>
            </div>
          </button>
        {:else}
          <div class="cmd-empty">Eşleşen komut bulunamadı</div>
        {/each}
      </div>
    </div>
  </div>
{/if}

<style>
  .cmd-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: var(--bg-primary);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 120px;
    z-index: 200;
    animation: fadeIn 0.15s ease-out;
  }

  .cmd-dialog {
    width: 540px;
    max-width: 90vw;
    background: var(--bg-surface);
    border: 1px solid var(--border-raw);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .cmd-header {
    display: flex;
    align-items: center;
    padding: 14px 18px;
    border-bottom: 1px solid var(--border-raw);
    gap: 12px;
    color: var(--text-muted);
  }

  .cmd-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: var(--text-primary);
    font-size: 15px;
    font-weight: 500;
    font-family: inherit;
  }

  .cmd-badge {
    font-size: 10px;
    font-weight: 700;
    padding: 2px 6px;
    background: var(--bg-panel);
    color: var(--text-muted);
  }

  .cmd-list {
    max-height: 320px;
    overflow-y: auto;
    padding: 8px;
  }

  .cmd-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    background: transparent;
    border: none;
    color: var(--text-primary);
    text-align: left;
    cursor: pointer;
    transition: background 0.1s ease;
  }

  .cmd-item.active {
    background: var(--bg-panel);
    border-left: 2px solid var(--accent-industrial);
  }

  .cmd-icon {
    font-size: 18px;
  }

  .cmd-text {
    display: flex;
    flex-direction: column;
  }

  .cmd-title {
    font-size: 13px;
    font-weight: 500;
  }

  .cmd-category {
    font-size: 10px;
    color: var(--text-muted);
  }

  .cmd-empty {
    padding: 24px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
  }
</style>
