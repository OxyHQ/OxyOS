// `appearance` (docs/adr/0058-the-platform-api.mdx, decision 1): what every application draws by,
// followed: light or dark (React Native's Appearance), the accent and reduce transparency (the
// session's drawing preferences, docs/adr/0019-settings.mdx), reduced motion (React Native's
// AccessibilityInfo), and the session's locale (docs/adr/0040-language-and-region.mdx): the BCP 47
// tag it tells every client, with its hour cycle (`es-ES-u-hc-h23`), which changes while the
// application runs.
import { followed, type Followed } from './followed.ts';
import type { Events, Subscription } from './session.ts';

/** The host's modules (OxyOSPreferences, OxyOSLocale) and React Native's own. */
export interface PreferencesModule {
  getState(): { accent?: string | null; reduceTransparency?: boolean } | null;
}
export interface LocaleModule {
  getLocale(): string;
}
export interface ColorSchemes {
  getColorScheme(): 'light' | 'dark' | null | undefined;
  addChangeListener(listener: (preferences: { colorScheme: 'light' | 'dark' | null | undefined }) => void): Subscription;
}
export interface MotionPreferences {
  isReduceMotionEnabled(): Promise<boolean>;
  addEventListener(event: 'reduceMotionChanged', listener: (enabled: boolean) => void): Subscription;
}

/** The session's drawing preferences beyond light and dark. */
export interface Drawing {
  /** The accent colour, which seeds Bloom's palette; null for the default. */
  readonly accent: string | null;
  readonly reduceTransparency: boolean;
}

const drawingOf = (told: { accent?: string | null; reduceTransparency?: boolean }): Drawing =>
  Object.freeze({ accent: told.accent || null, reduceTransparency: Boolean(told.reduceTransparency) });

export function createAppearance(
  modules: {
    preferences: PreferencesModule | null | undefined;
    locale: LocaleModule | null | undefined;
    colorSchemes: ColorSchemes | null | undefined;
    motion: MotionPreferences | null | undefined;
  },
  events: Events,
) {
  const { preferences, locale, colorSchemes, motion } = modules;

  // The accent and reduce transparency: only with a connection to the compositor, whose first
  // events say them (null without).
  const drawing = followed<Drawing>();
  const first = preferences?.getState();
  if (first) {
    drawing.tell(drawingOf(first));
    events.addListener('OxyOSPreferencesChanged', (next: Parameters<typeof drawingOf>[0]) => drawing.tell(drawingOf(next)));
  }

  // The session's locale: Intl's default, the tag the session last told.
  const localeTag = followed<string>();
  if (locale) {
    localeTag.tell(locale.getLocale());
    events.addListener('OxyOSLocaleChanged', () => localeTag.tell(locale.getLocale()));
  }

  // Light or dark, as React Native's Appearance has it (which the session's preference drives).
  const scheme = followed<'light' | 'dark'>();
  if (colorSchemes) {
    scheme.tell(colorSchemes.getColorScheme() === 'dark' ? 'dark' : 'light');
    colorSchemes.addChangeListener(({ colorScheme }) => scheme.tell(colorScheme === 'dark' ? 'dark' : 'light'));
  }

  // Reduced motion: asked as it is first followed, then told as it changes.
  let watchingMotion: Subscription | null = null;
  const reducedMotion = followed<boolean>({
    start() {
      if (!motion) return;
      watchingMotion = motion.addEventListener('reduceMotionChanged', (enabled) => reducedMotion.tell(Boolean(enabled)));
      motion.isReduceMotionEnabled().then(
        (enabled) => reducedMotion.tell(Boolean(enabled)),
        () => {},
      );
    },
    stop() {
      watchingMotion?.remove();
      watchingMotion = null;
    },
  });

  return {
    /** The accent and reduce transparency, followed; null without a compositor. */
    drawing: drawing as Followed<Drawing>,
    /** The session's locale, followed. */
    locale: localeTag as Followed<string>,
    /** Light or dark, followed. */
    colorScheme: scheme as Followed<'light' | 'dark'>,
    /** Whether the person asked for reduced motion, followed. */
    reducedMotion: reducedMotion as Followed<boolean>,
  };
}

export type Appearance = ReturnType<typeof createAppearance>;
