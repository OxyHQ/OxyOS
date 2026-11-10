/**
 * The session's locale as an application gets it (docs/adr/0040-language-and-region.mdx, decision
 * 4), for scenarios/locale.json: through the APIs it already uses, followed live.
 * - "I18n" is React Native's I18nManager.localeIdentifier, as Android gives it (`es_ES`), read once
 *   when the host started, as on Android.
 * - "Locale" and "Calendar" are expo-localization's getLocales()[0] and getCalendars()[0], through
 *   useLocales() and useCalendars(), which render again on its change events.
 * - "Intl", "Time" and "Plural" are Intl's default locale: DateTimeFormat's resolved locale, a
 *   fixed time's toLocaleTimeString() without a locale (in UTC), and PluralRules' categories of 1
 *   and of 2 as an ordinal, all with no locale given.
 * - "Session" is oxyos/locale.js's useSessionLocale(), what the system's applications give Bloom.
 * - "Events" counts expo-localization's onLocaleSettingsChanged and onCalendarSettingsChanged.
 * - "12-hour", "Mexico" and "English" ask the session to change a preference (`OxyOSSettings`, which
 *   the scenario grants this probe as the session grants Settings): the session keeps it and tells
 *   every application its new locale, this one among them.
 */
import { requireNativeModule } from 'expo';
import { useCalendars, useLocales } from 'expo-localization';
import * as React from 'react';
import { I18nManager, Pressable, Text, TurboModuleRegistry, View } from 'react-native';
import { useSessionLocale } from './oxyos/locale';

const T = Date.UTC(2026, 9, 29, 15, 4, 5);
// expo-localization's native module, whose events its hooks subscribe to (the package exports no
// listener functions of its own).
const localization = requireNativeModule<{
  addListener(event: string, listener: () => void): { remove(): void };
}>('ExpoLocalization');
const settings = TurboModuleRegistry.get<{ setPreference(key: string, value: string): number }>('OxyOSSettings');

function Button({ label, x, onPress }: { label: string; x: number; onPress: () => void }) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      style={{ position: 'absolute', left: x, top: 20, width: 140, height: 40, backgroundColor: '#2f6fe4', borderRadius: 8, justifyContent: 'center', alignItems: 'center' }}
    >
      <Text style={{ color: 'white' }}>{label}</Text>
    </Pressable>
  );
}

export default function LocaleProbe() {
  const [locale] = useLocales();
  const [calendar] = useCalendars();
  const session = useSessionLocale();
  const [events, setEvents] = React.useState({ locale: 0, calendar: 0 });
  React.useEffect(() => {
    const subscriptions = [
      localization.addListener('onLocaleSettingsChanged', () => setEvents((told) => ({ ...told, locale: told.locale + 1 }))),
      localization.addListener('onCalendarSettingsChanged', () =>
        setEvents((told) => ({ ...told, calendar: told.calendar + 1 })),
      ),
    ];
    return () => {
      for (const subscription of subscriptions) subscription.remove();
    };
  }, []);
  const rows = [
    `I18n: ${I18nManager.getConstants().localeIdentifier} ${I18nManager.isRTL ? 'rtl' : 'ltr'}`,
    `Locale: ${locale.languageTag} ${locale.languageCode} ${locale.regionCode} ${locale.languageScriptCode} ` +
      `${locale.textDirection} [${locale.decimalSeparator}] [${locale.digitGroupingSeparator}] ` +
      `${locale.measurementSystem} ${locale.temperatureUnit} ${locale.currencyCode}`,
    `Calendar: ${calendar.calendar} ${calendar.uses24hourClock ? '24h' : '12h'} ${calendar.firstWeekday} ${calendar.timeZone}`,
    `Intl: ${Intl.DateTimeFormat().resolvedOptions().locale}`,
    `Time: ${new Date(T).toLocaleTimeString(undefined, { timeZone: 'UTC' })}`,
    `Plural: ${new Intl.PluralRules().select(1)} ${new Intl.PluralRules(undefined, { type: 'ordinal' }).select(2)}`,
    `Session: ${session}`,
    `Events: ${events.locale} ${events.calendar}`,
  ];
  return (
    <View style={{ flex: 1, backgroundColor: 'white' }}>
      <Button label="12-hour" x={20} onPress={() => settings?.setPreference('hour_cycle', 'h12')} />
      <Button label="Mexico" x={180} onPress={() => settings?.setPreference('region', 'MX')} />
      <Button label="English" x={340} onPress={() => settings?.setPreference('language', 'en')} />
      {rows.map((row, index) => (
        <Text key={index} style={{ position: 'absolute', left: 20, top: 80 + index * 22 }}>
          {row}
        </Text>
      ))}
    </View>
  );
}
