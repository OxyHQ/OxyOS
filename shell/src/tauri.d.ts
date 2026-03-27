/** Tauri injects this global when running inside a webview. */
interface Window {
  __TAURI_INTERNALS__?: {
    metadata?: {
      currentWebview?: {
        label: string;
      };
    };
    [key: string]: unknown;
  };
}
