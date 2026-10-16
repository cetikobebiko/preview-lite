<script>
  import PasswordModal from "./PasswordModal.svelte";
  import AsyncPromptDialog from "./AsyncPromptDialog.svelte";

  let {
    ui,
    toastMsg,
    onPasswordSubmit,
    onPasswordCancel,
    asyncPrompt,
  } = $props();

  let passwordAttempt = $derived(
    ui.activeModal?.type === "password" ? ui.activeModal.props : null,
  );
</script>

{#if ui.isDraggingFileOver}
  <div class="drag-overlay">
    <div class="drag-box">
      <h3>// SÜRÜKLENEN_BELGEYİ_BIRAKIN</h3>
      <p>Mevcut belge kapatılacak ve yeni dosya yüklenecektir</p>
    </div>
  </div>
{/if}

{#if toastMsg}
  <div class="toast-notification">
    <span>// {toastMsg.toLocaleUpperCase("tr-TR")}</span>
  </div>
{/if}

<PasswordModal
  isOpen={Boolean(passwordAttempt)}
  fileName={passwordAttempt?.candidateName ?? ""}
  errorMsg={passwordAttempt?.error ?? ""}
  onsubmit={(password) => onPasswordSubmit(password, passwordAttempt?.attemptId)}
  oncancel={() => onPasswordCancel(passwordAttempt?.attemptId)}
/>

<AsyncPromptDialog coordinator={asyncPrompt} />

<style>
  .drag-overlay {
    position: fixed;
    inset: 0;
    z-index: 9999;
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
    background: var(--bg-primary);
    animation: fadeIn 0.15s ease-out;
  }

  .drag-box {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 40px 50px;
    text-align: center;
    border: 2px dashed var(--accent-industrial);
    background: var(--bg-surface);
  }

  .drag-box h3 {
    margin: 0 0 6px;
    color: var(--accent-industrial);
    font-size: 16px;
    font-weight: 700;
    text-transform: uppercase;
  }

  .drag-box p {
    margin: 0;
    color: var(--text-muted);
    font-size: 12px;
  }
</style>
