// `sounds` (docs/adr/0058-the-platform-api.mdx, decision 1; docs/adr/0044-system-sounds.mdx): the
// system's interface sounds. An application asks by meaning and the session plays its own sound
// theme's, as the person's settings and the output allow, citing the latest input it received.
import type { Spec } from '../specs/NativeOxyOSSounds.ts';
import { PlatformError } from './errors.ts';

/** The host's module (OxyOSSounds), as its spec says it. */
export type SoundsModule = Pick<Spec, 'getConstants' | 'play'>;

/** The sounds an application may ask for, by meaning. */
export const SYSTEM_SOUNDS = ['move-to-trash', 'empty-trash'] as const;
export type SystemSound = (typeof SYSTEM_SOUNDS)[number];

export function createSounds(native: SoundsModule | null | undefined) {
  const available = Boolean(native?.getConstants()?.available);
  return {
    available,
    /** Plays `sound`; rejects with `needsInput` where there was no input of the application's to
        cite (a sound is the answer to something the person did), `invalid` for no such sound. */
    async play(sound: SystemSound): Promise<void> {
      if (!native || !available) throw new PlatformError('sounds', 'unavailable');
      if (!(SYSTEM_SOUNDS as readonly string[]).includes(sound)) throw new PlatformError('sounds', 'invalid', String(sound));
      if (!native.play(sound)) throw new PlatformError('sounds', 'needsInput');
    },
  };
}

export type Sounds = ReturnType<typeof createSounds>;
