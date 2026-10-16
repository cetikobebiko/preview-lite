// On macOS the window controls are drawn over the webview (titleBarStyle
// "Overlay" in tauri.macos.conf.json), so the toolbar leaves room for them.
export const TRAFFIC_LIGHT_INSET_PX = 78;

export function usesOverlayTitleBar(nav = globalThis.navigator) {
  const platform = nav?.userAgentData?.platform || nav?.platform || "";
  return /mac/i.test(platform);
}
