// The host's module behind `application` (docs/adr/0058-the-platform-api.mdx, decision 1): the
// application this host runs and opening a URL elsewhere. The runtime's own, reached only by
// @oxyos/platform and the Expo modules the runtime implements (decision 2). React Native's codegen
// makes the C++ base the host implements from this spec (decision 5), so a method the spec names
// and the host lacks, or a type that differs, does not compile.
import type { CodegenTypes, TurboModule } from 'react-native';
import { TurboModuleRegistry } from 'react-native';

export type ApplicationConstants = {
  /** The id the session gave it (its registry entry's), null where it did not start it. */
  id: string | null;
  /** Whether the session started it from its records (a launch record), not a claim. */
  recorded: boolean;
  /** The module it registers with AppRegistry. */
  module: string;
  /** Its embedded app config (expo-constants' manifest), if any. */
  manifest: CodegenTypes.UnsafeObject | null;
  dataDirectory: string | null;
  cacheDirectory: string | null;
  /** The machine's name and the system's version (expo-constants). */
  deviceName: string;
  systemVersion: string;
  /** What the session started it to open (its launch record's: a file or a URL of its scheme). */
  launchURL: string | null;
};

/** What came of opening a URL: opened, or refused with the platform's word and its words. */
export type OpenAnswer = {
  opened: boolean;
  refused: { reason: string; message: string } | null;
};

export interface Spec extends TurboModule {
  getConstants(): ApplicationConstants;
  /** Asks the session to open `url` with the application whose entry declares its scheme, citing
      the person's latest press or key; never a file. */
  open(url: string): Promise<OpenAnswer>;
}

export default TurboModuleRegistry.get<Spec>('OxyOSApplication');
