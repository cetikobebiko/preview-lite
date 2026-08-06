import { getContext } from "svelte";

export const ASYNC_PROMPT_CONTEXT = Symbol("async-prompt");

export function createAsyncPromptCoordinator() {
  let state = $state({ active: null });
  let pendingResolve = null;
  let sequence = 0;

  function request({ title, label, initialValue = "", placeholder = "" }) {
    if (pendingResolve) pendingResolve(null);
    return new Promise((resolve) => {
      pendingResolve = resolve;
      state.active = {
        id: ++sequence,
        title,
        label,
        initialValue,
        placeholder,
      };
    });
  }

  function settle(value) {
    const resolve = pendingResolve;
    pendingResolve = null;
    state.active = null;
    resolve?.(value);
  }

  return {
    get active() {
      return state.active;
    },
    request,
    submit(value) {
      settle(String(value));
    },
    cancel() {
      settle(null);
    },
  };
}

export function requireAsyncPromptCoordinator() {
  const coordinator = getContext(ASYNC_PROMPT_CONTEXT);
  if (!coordinator) throw new Error("Async prompt context is unavailable");
  return coordinator;
}
