/** Tauri injects this global when running inside a webview. */
interface Window {
  __TAURI_INTERNALS__?: Record<string, unknown>;
}
