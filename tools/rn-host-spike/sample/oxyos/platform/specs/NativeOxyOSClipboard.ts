// The host's module behind `pasteboard` (docs/adr/0007-clipboard.mdx): the runtime's own, reached
// only by @oxyos/platform (docs/adr/0058-the-platform-api.mdx, decision 2). React Native's codegen
// makes the C++ base the host implements from this spec (decision 5), so a method the spec names
// and the host lacks, or a type that differs, does not compile. Each call cites the latest key or
// press the application received, and throws at once where the application has no window.
import type { TurboModule } from 'react-native';
import { TurboModuleRegistry } from 'react-native';

export interface Spec extends TurboModule {
  hasString(): Promise<boolean>;
  /** Whether the compositor took the copy. */
  setString(text: string): Promise<boolean>;
  /**
   * A password or a code (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 8): marked
   * sensitive, so no clipboard manager keeps it, and cleared after 90 seconds if still there
   * (docs/adr/0007-clipboard.mdx, decision 6). Whether the compositor took it; false for nothing.
   */
  setSensitiveString(text: string): Promise<boolean>;
  /** The text, or "" where there is none or the compositor refused the paste. */
  getString(): Promise<string>;
  addListener(eventName: string): void;
  removeListeners(count: number): void;
}

export default TurboModuleRegistry.get<Spec>('OxyOSClipboard');
