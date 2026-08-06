import { listen as tauriListen } from "@tauri-apps/api/event";

export const listen = (...args) => tauriListen(...args);
