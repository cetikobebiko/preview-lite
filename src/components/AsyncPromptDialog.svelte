<script>
  let { coordinator } = $props();

  let value = $state("");
  let inputRef = $state();

  $effect(() => {
    const request = coordinator.active;
    if (!request) return;
    value = request.initialValue ?? "";
    queueMicrotask(() => inputRef?.focus());
  });

  function submit(event) {
    event.preventDefault();
    coordinator.submit(value);
  }
</script>

{#if coordinator.active}
  <dialog
    open
    class="async-prompt-backdrop"
    aria-labelledby="async-prompt-title"
    oncancel={(event) => {
      event.preventDefault();
      coordinator.cancel();
    }}
  >
    <form
      class="async-prompt-dialog glass-panel"
      onsubmit={submit}
    >
      <h2 id="async-prompt-title">{coordinator.active.title}</h2>
      <label for="async-prompt-input">{coordinator.active.label}</label>
      <input
        id="async-prompt-input"
        bind:this={inputRef}
        bind:value
        placeholder={coordinator.active.placeholder}
      />
      <div class="async-prompt-actions">
        <button type="button" onclick={() => coordinator.cancel()}>İptal</button>
        <button type="submit">Tamam</button>
      </div>
    </form>
  </dialog>
{/if}

<style>
  .async-prompt-backdrop {
    position: fixed;
    inset: 0;
    z-index: 10000;
    display: grid;
    place-items: center;
    width: 100%;
    max-width: none;
    height: 100%;
    max-height: none;
    margin: 0;
    padding: 0;
    border: 0;
    background: rgb(0 0 0 / 55%);
  }

  .async-prompt-dialog {
    width: min(28rem, calc(100vw - 2rem));
    display: grid;
    gap: 0.75rem;
    padding: 1rem;
    border: 1px solid rgb(255 255 255 / 14%);
    border-radius: 0.75rem;
    background: #18181b;
  }

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  label {
    color: #d4d4d8;
    font-size: 0.85rem;
  }

  input {
    width: 100%;
    border: 1px solid rgb(255 255 255 / 18%);
    border-radius: 0.45rem;
    padding: 0.6rem 0.7rem;
    color: white;
    background: #09090b;
  }

  .async-prompt-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  button {
    border: 1px solid rgb(255 255 255 / 15%);
    border-radius: 0.45rem;
    padding: 0.45rem 0.75rem;
  }
</style>
