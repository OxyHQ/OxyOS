/**
 * Shared Tauri IPC helper.
 * All calls gracefully return null when running in a regular browser,
 * so the shell works in both demo (browser) and native (Tauri) modes.
 */

export async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T | null> {
  try {
    if (window.__TAURI_INTERNALS__) {
      const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
      return await tauriInvoke<T>(cmd, args);
    }
  } catch {
    // Browser mode — silent fallback
  }
  return null;
}

export function isNative(): boolean {
  return !!window.__TAURI_INTERNALS__;
}

/** Convert an absolute file path to a Tauri asset protocol URL */
export function assetUrl(filePath: string): string {
  if (!filePath || !filePath.startsWith("/")) return filePath;
  if (!isNative()) return filePath;
  // Tauri v2 asset protocol
  return window.__TAURI_INTERNALS__
    ? `http://asset.localhost/${filePath}`
    : filePath;
}
