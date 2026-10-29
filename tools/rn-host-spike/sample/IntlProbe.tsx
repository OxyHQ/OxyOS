/**
 * ECMA-402's Intl on OxyOS (intl.h, crates/oxyos-intl), for scenarios/intl.json: dates and numbers
 * formatted for several locales and time zones, a date's parts, an option OxyOS refuses with a
 * RangeError rather than formatting wrongly, and a constructor it does not provide left undefined.
 * The time is fixed (2026-10-29 15:04:05 UTC) so every row is the same on every machine.
 */
import * as React from 'react';
import { Text, View } from 'react-native';

const T = Date.UTC(2026, 9, 29, 15, 4, 5);

function attempt(run: () => string): string {
  try {
    return run();
  } catch (error) {
    return `refused: ${(error as Error).name}`;
  }
}

const ROWS: Record<string, () => string> = {
  'es date': () =>
    new Intl.DateTimeFormat('es-ES', { day: 'numeric', month: 'long', year: 'numeric', timeZone: 'UTC' }).format(T),
  'en medium short': () =>
    new Intl.DateTimeFormat('en-US', { dateStyle: 'medium', timeStyle: 'short', timeZone: 'UTC' }).format(T),
  'es weekday': () =>
    new Date(T).toLocaleDateString('es', { weekday: 'long', day: 'numeric', month: 'long', timeZone: 'UTC' }),
  'gb madrid': () =>
    new Date(T).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', timeZone: 'Europe/Madrid' }),
  'de short month': () => new Intl.DateTimeFormat('de-DE', { day: 'numeric', month: 'short', timeZone: 'UTC' }).format(T),
  'es parts': () =>
    new Intl.DateTimeFormat('es-ES', { day: 'numeric', month: 'long', year: 'numeric', timeZone: 'UTC' })
      .formatToParts(T)
      .map((part) => part.type)
      .join(','),
  'de number': () => new Intl.NumberFormat('de-DE').format(1234.5),
  'es eur': () => new Intl.NumberFormat('es-ES', { style: 'currency', currency: 'EUR' }).format(1234.5),
  'en usd': () => new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).format(-1234.5),
  'en percent': () => new Intl.NumberFormat('en-US', { style: 'percent' }).format(0.256),
  'fr max0': () => (1234.567).toLocaleString('fr-FR', { maximumFractionDigits: 0 }),
  'en sig': () => new Intl.NumberFormat('en-US', { maximumSignificantDigits: 3 }).format(123456),
  compact: () => new Intl.NumberFormat('en-US', { notation: 'compact' }).format(1234),
  collator: () => typeof (Intl as unknown as Record<string, unknown>).Collator,
  'time zone': () => (Intl.DateTimeFormat().resolvedOptions().timeZone ? 'known' : 'unknown'),
};

export default function IntlProbe() {
  return (
    <View style={{ flex: 1, padding: 16, gap: 4, backgroundColor: '#fff' }}>
      {Object.entries(ROWS).map(([name, run]) => (
        <Text key={name}>{`${name}: ${attempt(run)}`}</Text>
      ))}
    </View>
  );
}
