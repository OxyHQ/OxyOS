// `pasteboard` (docs/adr/0058-the-platform-api.mdx, decision 1; docs/adr/0007-clipboard.mdx):
// copying and pasting plain text on the system's pasteboard. Every application may, citing an
// input: the compositor takes a copy or gives a paste only to the application with the keyboard
// focus that acted just before (one of its latest inputs).
import { PlatformError } from './errors.ts';
import type { Events } from './session.ts';
import type { CallOptions } from './frameworks.ts';
import { hostCall } from './host.ts';

/** The host's module (OxyOSClipboard). Its calls throw where the application has no window. */
export interface ClipboardModule {
  hasString(): Promise<boolean>;
  setString(text: string): Promise<boolean>;
  getString(): Promise<string>;
}

const unavailable = () => new PlatformError('pasteboard', 'unavailable');
/** A host call of the pasteboard's (its calls throw at once where the application has no window). */
const call = <T>(start: () => Promise<T>, options?: CallOptions) => hostCall('pasteboard', start, options);

export function createPasteboard(native: ClipboardModule | null | undefined, events: Events) {
  return {
    available: Boolean(native),
    /** Whether it holds text, without reading it. */
    hasText: (options?: CallOptions) => (native ? call(() => native.hasString(), options) : Promise.reject(unavailable())),
    /** Its text: "" where it holds none, or where the compositor refused the paste (no recent
        input of this application's to cite: a paste is never refused louder than that). */
    readText: (options?: CallOptions) => (native ? call(() => native.getString(), options) : Promise.reject(unavailable())),
    /** Puts `text` on it; rejects with `needsInput` where the compositor refused it. */
    writeText: async (text: string, options?: CallOptions): Promise<void> => {
      if (!native) throw unavailable();
      const copied = await call(() => native.setString(String(text)), options);
      if (!copied) throw new PlatformError('pasteboard', 'needsInput');
    },
    /** Its content changed (another application copied): whether it now holds text. */
    onChange(listener: (state: { hasText: boolean }) => void): () => void {
      const subscription = events.addListener('OxyOSClipboardChanged', ({ text }: { text?: boolean }) =>
        listener({ hasText: Boolean(text) }),
      );
      return () => subscription.remove();
    },
  };
}

export type Pasteboard = ReturnType<typeof createPasteboard>;
