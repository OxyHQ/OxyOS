// The host's module behind `sounds` (docs/adr/0044-system-sounds.mdx): the runtime's own, reached
// only by @oxyos/platform (docs/adr/0058-the-platform-api.mdx, decision 2). React Native's codegen
// makes the C++ base the host implements from this spec (decision 5), so a method the spec names
// and the host lacks, or a type that differs, does not compile.
import type { TurboModule } from 'react-native';
import { TurboModuleRegistry } from 'react-native';

export interface Spec extends TurboModule {
  getConstants(): { available: boolean };
  /** Plays the system's sound `name` (Bloom's names: `move-to-trash`), citing the latest input:
      whether it was asked for (false: no input of the application's to cite, no such sound). */
  play(name: string): boolean;
}

export default TurboModuleRegistry.get<Spec>('OxyOSSounds');
