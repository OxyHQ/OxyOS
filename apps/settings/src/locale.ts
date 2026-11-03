/**
 * Language & Region (docs/adr/0040-language-and-region.mdx, decisions 1, 2 and 7): the session's
 * locale as the session computes it from the person's choices and the system's locale, and what
 * Settings' page offers and shows of it.
 */
import type { HourCycle, LanguagePreference, Preferences } from "./model";
import type { Messages, MessagesLanguage } from "./messages";

/** The languages OxyOS speaks, each named in itself, as macOS lists them. */
export const LANGUAGES = [
  { value: "en", name: "English" },
  { value: "es", name: "Español" },
] as const satisfies readonly { value: Exclude<LanguagePreference, "">; name: string }[];

/** The regions the page offers (CLDR's codes); a region chosen elsewhere is offered too. */
export const REGIONS = [
  "ES",
  "MX",
  "AR",
  "CO",
  "CL",
  "PE",
  "US",
  "GB",
  "IE",
  "CA",
  "AU",
  "DE",
  "FR",
  "IT",
  "PT",
  "NL",
] as const;

/** A language's likely region, for a locale that names none. */
const LIKELY_REGION: Record<MessagesLanguage, string> = { en: "US", es: "ES" };

/**
 * The system's language and region from its locale (a BCP 47 tag, "es-ES", or POSIX's
 * "es_ES.UTF-8"): its language when OxyOS speaks it, else English; its territory, else the
 * language's likely region.
 */
export function systemLanguageRegion(locale: string): { language: MessagesLanguage; region: string } {
  const tag = locale.split(/[.@]/)[0] ?? "";
  const subtags = tag.split(/[-_]/);
  const language: MessagesLanguage = subtags[0]?.toLowerCase() === "es" ? "es" : "en";
  // The region is before any extension's singleton ("-u-…").
  const end = subtags.findIndex((subtag, at) => at > 0 && subtag.length === 1);
  const region = subtags
    .slice(1, end === -1 ? undefined : end)
    .find((subtag) => /^[A-Za-z]{2}$/.test(subtag))
    ?.toUpperCase();
  return { language, region: region ?? LIKELY_REGION[language] };
}

/** The language and region in effect: the person's choices, else the system's. */
export function effectiveLanguageRegion(
  preferences: Pick<Preferences, "language" | "region">,
  systemLocale: string,
): { language: MessagesLanguage; region: string } {
  const system = systemLanguageRegion(systemLocale);
  return {
    language: preferences.language || system.language,
    region: /^[A-Z]{2}$/.test(preferences.region) ? preferences.region : system.region,
  };
}

/**
 * The session's locale, one BCP 47 tag (decision 2): the language, the region and, only when the
 * person chose one, the hour cycle as a Unicode extension: "es-ES", "en-ES", "es-MX-u-hc-h12".
 */
export function sessionLocale(
  preferences: Pick<Preferences, "language" | "region" | "hourCycle">,
  systemLocale: string,
): string {
  const { language, region } = effectiveLanguageRegion(preferences, systemLocale);
  const cycle = preferences.hourCycle === "auto" ? "" : `-u-hc-${preferences.hourCycle}`;
  return `${language}-${region}${cycle}`;
}

/** The hour cycle a locale formats with: 12 hours (`h12`, also for `h11`) or 24 (`h23`, `h24`). */
export function hourCycleOf(locale: string): Exclude<HourCycle, "auto"> {
  const cycle = new Intl.DateTimeFormat(locale, { hour: "numeric" }).resolvedOptions().hourCycle;
  return cycle === "h12" || cycle === "h11" ? "h12" : "h23";
}

/** The region's own hour cycle, in a language: what Automatic gives. */
export function regionHourCycle(language: MessagesLanguage, region: string): Exclude<HourCycle, "auto"> {
  return hourCycleOf(`${language}-${region}`);
}

/** The moment and the number the page's sample formats: a Tuesday afternoon, so 12 and 24 hours
    differ, and a number with thousands and decimals. Local time, as the clock shows it. */
export const SAMPLE_DATE = new Date(2027, 0, 5, 14, 45);
export const SAMPLE_NUMBER = 1234567.89;

/** A date, a time and a number as a locale formats them. */
export function localeSample(
  locale: string,
  at: Date = SAMPLE_DATE,
  number: number = SAMPLE_NUMBER,
): { date: string; time: string; number: string } {
  return {
    date: new Intl.DateTimeFormat(locale, { dateStyle: "full" }).format(at),
    time: new Intl.DateTimeFormat(locale, { timeStyle: "short" }).format(at),
    number: new Intl.NumberFormat(locale).format(number),
  };
}

type Item<T extends string> = { value: T; label: string };
type Words = Messages["languageRegion"];

/** Language's choices: Automatic, named with the system's language in the session's, then each
    language OxyOS speaks, named in itself. */
export function languageItems(words: Words, systemLocale: string): Item<LanguagePreference>[] {
  const system = systemLanguageRegion(systemLocale).language;
  return [
    { value: "", label: words.automatic(words.languageNames[system]) },
    ...LANGUAGES.map((language) => ({ value: language.value, label: language.name })),
  ];
}

/** A region's name in the session's language, else its code. */
export function regionName(words: Words, region: string): string {
  return words.regionNames[region] ?? region;
}

/** Region's choices: Automatic, named with the system's region, then the regions by name in the
    session's language, the one chosen among them even when the page does not list it. */
export function regionItems(words: Words, systemLocale: string, chosen: string): Item<string>[] {
  const system = systemLanguageRegion(systemLocale).region;
  const codes: string[] = [...REGIONS];
  if (/^[A-Z]{2}$/.test(chosen) && !codes.includes(chosen)) codes.push(chosen);
  const named = codes.map((code) => ({ value: code, label: regionName(words, code) }));
  // Every name starts with an unaccented capital, so code units sort them as people read them.
  named.sort((a, b) => (a.label < b.label ? -1 : a.label > b.label ? 1 : 0));
  return [{ value: "", label: words.automatic(regionName(words, system)) }, ...named];
}

/** Time format's choices: Automatic, named with the region's hour cycle, then 12 and 24 hours. */
export function timeFormatItems(
  words: Words,
  preferences: Pick<Preferences, "language" | "region">,
  systemLocale: string,
): Item<HourCycle>[] {
  const { language, region } = effectiveLanguageRegion(preferences, systemLocale);
  const regions = regionHourCycle(language, region);
  return [
    { value: "auto", label: words.automatic(regions === "h12" ? words.hour12 : words.hour24) },
    { value: "h12", label: words.hour12 },
    { value: "h23", label: words.hour24 },
  ];
}
