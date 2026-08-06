<script>
  import { createEventDispatcher, onMount } from "svelte";

  export let isOpen = false;
  export let fileName = "";
  export let errorMsg = "";

  const dispatch = createEventDispatcher();
  let password = "";
  let inputRef;

  $: if (isOpen && inputRef) {
    setTimeout(() => inputRef.focus(), 50);
  }

  function handleSubmit() {
    if (!password) return;
    dispatch("submit", { password });
    // Güvenlik: Parolayı Svelte state'inden hemen sil
    password = "";
  }

  function handleCancel() {
    password = "";
    dispatch("cancel");
  }

  function handleKeydown(e) {
    if (e.key === "Enter") {
      handleSubmit();
    } else if (e.key === "Escape") {
      handleCancel();
    }
  }
</script>

{#if isOpen}
  <div class="modal-overlay">
    <div class="modal-box">
      <div class="modal-header">
        <h3>GÜVENLİK_KONTROLÜ</h3>
      </div>
      <div class="modal-body">
        <p class="desc">
          <strong>{fileName}</strong> dosyası parola ile korunuyor. İçeriği görüntülemek için lütfen parolayı girin.
        </p>

        <input
          bind:this={inputRef}
          type="password"
          bind:value={password}
          on:keydown={handleKeydown}
          placeholder="Parolayı girin..."
          class="password-input {errorMsg ? 'error' : ''}"
        />

        {#if errorMsg}
          <div class="error-msg">{errorMsg}</div>
        {/if}
      </div>

      <div class="modal-footer">
        <button class="btn btn-secondary" on:click={handleCancel}>İptal</button>
        <button class="btn btn-primary" on:click={handleSubmit} disabled={!password}>Kilidi Aç</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-overlay {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: rgba(0, 0, 0, 0.7);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 9999;
  }
  .modal-box {
    width: 400px;
    background: var(--bg-surface, #111827);
    border: 1px solid var(--border-color, #1f2937);
    border-radius: 8px;
    box-shadow: 0 10px 25px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    font-family: var(--font-sans, "Inter", sans-serif);
  }
  .modal-header {
    background: rgba(255, 255, 255, 0.05);
    padding: 12px 16px;
    border-bottom: 1px solid var(--border-color, #1f2937);
  }
  .modal-header h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 700;
    color: #eab308; /* Yellow/Gold for security */
    letter-spacing: 0.05em;
  }
  .modal-body {
    padding: 20px 16px;
  }
  .desc {
    margin: 0 0 16px 0;
    font-size: 14px;
    color: #d1d5db;
    line-height: 1.5;
  }
  .desc strong {
    color: #f3f4f6;
  }
  .password-input {
    width: 100%;
    background: #1f2937;
    border: 1px solid #374151;
    color: #f3f4f6;
    padding: 10px 12px;
    border-radius: 4px;
    font-size: 14px;
    outline: none;
    transition: all 0.2s;
    box-sizing: border-box;
  }
  .password-input:focus {
    border-color: #3b82f6;
    box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.2);
  }
  .password-input.error {
    border-color: #ef4444;
  }
  .error-msg {
    margin-top: 8px;
    font-size: 12px;
    color: #ef4444;
  }
  .modal-footer {
    padding: 12px 16px;
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    background: rgba(0, 0, 0, 0.2);
    border-top: 1px solid var(--border-color, #1f2937);
  }
  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 4px;
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn-secondary {
    background: transparent;
    color: #9ca3af;
  }
  .btn-secondary:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.1);
    color: #f3f4f6;
  }
  .btn-primary {
    background: #3b82f6;
    color: white;
  }
  .btn-primary:hover:not(:disabled) {
    background: #2563eb;
  }
</style>
