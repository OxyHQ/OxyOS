import type { ShellSnapshot, ShellWindow } from "./model";

/** An application's windows as App Exposé shows them: its open ones, the one used last first (the
    frontmost, as a Mac's App Exposé leads with it), and its minimized ones apart. Hidden ones
    (Super+H) are open ones: choosing one shows its application again. */
export function exposedWindows(
  snapshot: Pick<ShellSnapshot, "windows">,
  appId: string,
): { open: ShellWindow[]; minimized: ShellWindow[] } {
  const mine = snapshot.windows
    // Its own, as `belongsTo` has it: a window the session did not start is no application's.
    .filter((window) => window.launched !== false && window.appId === appId)
    .slice()
    .sort((a, b) => (b.used ?? 0) - (a.used ?? 0));
  return {
    open: mine.filter((window) => !window.minimized),
    minimized: mine.filter((window) => window.minimized),
  };
}
