import { listen as tauriListen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";

export const listen = (...args) => tauriListen(...args);
/** Subscribes to native file drag-enter/over/leave/drop for the current webview. */
export const onDragDropEvent = (handler) => getCurrentWebview().onDragDropEvent(handler);
