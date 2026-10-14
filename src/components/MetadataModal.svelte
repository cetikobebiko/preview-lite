<script>
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
  <div class="meta-overlay" onclick={close} role="presentation" tabindex="-1">
    <div class="meta-dialog" onclick={(event) => event.stopPropagation()} role="presentation">
      <div class="meta-header">
        <div style="display: flex; align-items: center; gap: 8px;">
          <span style="font-size: 20px;">🏷️</span>
          <div>
            <h3 class="meta-title">PDF Meta Verileri (Salt Okunur)</h3>
            <p class="meta-subtitle">PDFium motoru metadata yazma C API'si sunmamaktadır</p>
          </div>
        </div>
        <button class="close-btn" onclick={close}>✕</button>
      </div>

      <div class="meta-body">
        <div class="unsupported-banner">
          ⚠️ PDFium C API metin/alt-text yazma desteği sunmamaktadır; metadata düzenleme desteklenmiyor.
        </div>

        <div class="form-group">
          <label for="meta-title">📌 Belge Başlığı (Title):</label>
          <input id="meta-title" type="text" class="meta-input" bind:value={metadata.title} readonly disabled />
        </div>

        <div class="form-group">
          <label for="meta-author">✍️ Yazar (Author):</label>
          <input id="meta-author" type="text" class="meta-input" bind:value={metadata.author} readonly disabled />
        </div>

        <div class="form-group">
          <label for="meta-subject">🏷️ Konu (Subject):</label>
          <input id="meta-subject" type="text" class="meta-input" bind:value={metadata.subject} readonly disabled />
        </div>

        <div class="form-group">
          <label for="meta-keywords">🔑 Anahtar Kelimeler (Keywords):</label>
          <input id="meta-keywords" type="text" class="meta-input" bind:value={metadata.keywords} readonly disabled />
        </div>

        <div class="form-group">
          <label for="meta-creator">⚙️ Oluşturan Uygulama (Creator):</label>
          <input id="meta-creator" type="text" class="meta-input" bind:value={metadata.creator} readonly disabled />
        </div>
      </div>

      <div class="meta-footer">
        <button class="cancel-btn" onclick={close}>Kapat</button>
        <button class="save-btn" disabled title="PDFium C API metin/alt-text yazma desteği sunmamaktadır">
          Kaydet (Desteklenmiyor)
        </button>
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

  .unsupported-banner {
    background: rgba(234, 179, 8, 0.15);
    border: 1px solid rgba(234, 179, 8, 0.4);
    color: #eab308;
    padding: 10px 14px;
    border-radius: 4px;
    font-size: 12px;
    line-height: 1.4;
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
    opacity: 0.7;
    cursor: not-allowed;
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
    background: var(--border-raw);
    border: none;
    color: var(--text-muted);
    padding: 6px 16px;
    cursor: not-allowed;
    font-size: 13px;
    font-weight: 500;
    opacity: 0.6;
  }
</style>
