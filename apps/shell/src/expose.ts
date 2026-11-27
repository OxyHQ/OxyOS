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

/** A window's title as Mission Control and App Exposé show it: "(Not Responding)" after it while
    its application does not answer (docs/adr/0067-hangs-and-force-quit.mdx), as a Mac's Force
    Quit and Windows' title bars say it. */
export function shownTitle(
  window: { title: string; responding?: boolean },
  words: { notRespondingTitle: (title: string) => string },
): string {
  return window.responding === false ? words.notRespondingTitle(window.title) : window.title;
}
