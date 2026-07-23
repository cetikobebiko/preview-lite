<script>
  import { createEventDispatcher } from "svelte";

  export let isOpen = false;
  export let metadata = {
    title: "",
    author: "",
    subject: "",
    keywords: "",
    creator: "",
  };

  const dispatch = createEventDispatcher();

  function close() {
    dispatch("close");
  }

  function handleSave() {
    dispatch("save", metadata);
    close();
  }
</script>

{#if isOpen}
  <div class="meta-overlay" on:click={close} role="presentation" tabindex="-1">
    <div class="meta-dialog" on:click|stopPropagation role="presentation">
      <div class="meta-header">
        <div style="display: flex; align-items: center; gap: 8px;">
          <span style="font-size: 20px;">🏷️</span>
          <div>
            <h3 class="meta-title">PDF Meta Veri Düzenleyici</h3>
            <p class="meta-subtitle">Belge başlık, yazar ve anahtar kelime bilgilerini güncelleyin</p>
          </div>
        </div>
        <button class="close-btn" on:click={close}>✕</button>
      </div>

      <div class="meta-body">
        <div class="form-group">
          <label for="meta-title">📌 Belge Başlığı (Title):</label>
          <input id="meta-title" type="text" class="meta-input" bind:value={metadata.title} placeholder="Örn: Yıllık Faaliyet Raporu" />
        </div>

        <div class="form-group">
          <label for="meta-author">✍️ Yazar (Author):</label>
          <input id="meta-author" type="text" class="meta-input" bind:value={metadata.author} placeholder="Örn: Ahmet Yılmaz" />
        </div>

        <div class="form-group">
          <label for="meta-subject">🏷️ Konu (Subject):</label>
          <input id="meta-subject" type="text" class="meta-input" bind:value={metadata.subject} placeholder="Örn: Finansal Sunum" />
        </div>

        <div class="form-group">
          <label for="meta-keywords">🔑 Anahtar Kelimeler (Keywords):</label>
          <input id="meta-keywords" type="text" class="meta-input" bind:value={metadata.keywords} placeholder="Örn: rapor, pdf, finans, 2026" />
        </div>

        <div class="form-group">
          <label for="meta-creator">⚙️ Oluşturan Uygulama (Creator):</label>
          <input id="meta-creator" type="text" class="meta-input" bind:value={metadata.creator} placeholder="Örn: Preview Lite macOS" />
        </div>
      </div>

      <div class="meta-footer">
        <button class="cancel-btn" on:click={close}>İptal</button>
        <button class="save-btn" on:click={handleSave}>Kaydet & Güncelle</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .meta-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: var(--bg-primary);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 180;
    animation: fadeIn 0.15s ease-out;
  }

  .meta-dialog {
    width: 480px;
    max-width: 90vw;
    background: var(--bg-surface);
    border: 1px solid var(--border-raw);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .meta-header {
    padding: 14px 18px;
    border-bottom: 1px solid var(--border-raw);
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: var(--bg-panel);
  }

  .meta-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .meta-subtitle {
    font-size: 11px;
    color: var(--text-muted);
  }

  .close-btn {
    background: transparent;
    border: none;
    color: var(--text-muted);
    font-size: 16px;
    cursor: pointer;
  }

  .meta-body {
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .form-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  label {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-muted);
  }

  .meta-input {
    background: var(--bg-primary);
    border: 1px solid var(--border-raw);
    padding: 8px 12px;
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
  }

  .meta-input:focus {
    border-color: var(--accent-industrial);
    outline: 1px solid var(--accent-industrial);
  }

  .meta-footer {
    padding: 12px 18px;
    border-top: 1px solid var(--border-raw);
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
    background: var(--bg-panel);
  }

  .cancel-btn {
    background: transparent;
    border: 1px solid var(--border-raw);
    color: var(--text-muted);
    padding: 6px 14px;
    cursor: pointer;
    font-size: 13px;
  }

  .save-btn {
    background: var(--accent-industrial);
    border: none;
    color: white;
    padding: 6px 16px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
  }
</style>
