import { ask as tauriAsk, open as tauriOpen, save as tauriSave } from "@tauri-apps/plugin-dialog";

export const ask = (...args) => tauriAsk(...args);
export const open = (...args) => tauriOpen(...args);
export const save = (...args) => tauriSave(...args);
