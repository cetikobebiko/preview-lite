<script>
  import FileDownIcon from "@lucide/svelte/icons/file-down";
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
      <FileDownIcon class="mb-3 size-10 text-primary" strokeWidth={1.5} />
      <h3>Belgeyi bırakın</h3>
      <p>Mevcut belge kapatılacak ve yeni dosya açılacak.</p>
    </div>
  </div>
{/if}

{#if toastMsg}
  <div class="toast-notification" role="status">
    <span>{toastMsg}</span>
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
    padding: 24px;
    pointer-events: none;
    background: color-mix(in srgb, var(--background) 70%, transparent);
    animation: fadeIn 0.15s ease-out;
  }

  .drag-box {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 36px 48px;
    text-align: center;
    border: 2px dashed color-mix(in srgb, var(--primary) 60%, transparent);
    border-radius: calc(var(--radius) * 2);
    background: var(--card);
    box-shadow: 0 8px 32px rgb(0 0 0 / 0.12);
  }

  .drag-box h3 {
    margin: 0 0 4px;
    font-size: 15px;
    font-weight: 600;
  }

  .drag-box p {
    margin: 0;
    color: var(--muted-foreground);
    font-size: 12px;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }
</style>
