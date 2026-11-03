/**
 * The session's locale for an application's root (docs/adr/0040-language-and-region.mdx, decision
 * 5): the BCP 47 tag the session tells every client, with its hour cycle always said (`es-ES-u-hc-h23`,
 * `es-MX-u-hc-h12`), for `BloomProvider locale=`. Built on expo-localization, as any Expo
 * application reads the platform's locale (oxyos/expo-adapter.js gives it the session's): the
 * component renders again when the session tells another.
 */
import { useCalendars, useLocales } from 'expo-localization';

export function useSessionLocale() {
  const [locale] = useLocales();
  const [calendar] = useCalendars();
  return `${locale.languageTag}-u-hc-${calendar.uses24hourClock ? 'h23' : 'h12'}`;
}
