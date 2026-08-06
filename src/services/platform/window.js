import { getCurrentWindow as getTauriWindow } from "@tauri-apps/api/window";

export const getCurrentWindow = () => getTauriWindow();
