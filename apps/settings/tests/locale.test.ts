import { test } from "node:test";
import assert from "node:assert/strict";
import {
  effectiveLanguageRegion,
  hourCycleOf,
  languageItems,
  localeSample,
  REGIONS,
  regionHourCycle,
  regionItems,
  sessionLocale,
  systemLanguageRegion,
  timeFormatItems,
} from "../src/locale.ts";
import { CATALOGUES, messagesFor, messagesLanguage } from "../src/messages.ts";
import { PREFERENCE_KEYS, preferenceText } from "../src/model.ts";

const en = messagesFor("en");
const es = messagesFor("es");
const none = { language: "", region: "", hourCycle: "auto" } as const;

test("the system's language and region from its locale, as the session reads them", () => {
  assert.deepEqual(systemLanguageRegion("es-ES"), { language: "es", region: "ES" });
  assert.deepEqual(systemLanguageRegion("es_MX.UTF-8"), { language: "es", region: "MX" });
  assert.deepEqual(systemLanguageRegion("en_GB.UTF-8@euro"), { language: "en", region: "GB" });
  // A language OxyOS does not speak is English, in its own territory.
  assert.deepEqual(systemLanguageRegion("fr-FR"), { language: "en", region: "FR" });
  // No territory: the language's likely region.
  assert.deepEqual(systemLanguageRegion("es"), { language: "es", region: "ES" });
  assert.deepEqual(systemLanguageRegion("C"), { language: "en", region: "US" });
  assert.deepEqual(systemLanguageRegion(""), { language: "en", region: "US" });
  // A script before the region, and an extension after it.
  assert.deepEqual(systemLanguageRegion("es-Latn-AR"), { language: "es", region: "AR" });
  assert.deepEqual(systemLanguageRegion("es-u-hc-h12"), { language: "es", region: "ES" });
  assert.deepEqual(systemLanguageRegion("en-US-u-hc-h23"), { language: "en", region: "US" });
});

test("the session's locale: the choices, else the system's, and the hour cycle only when chosen", () => {
  assert.equal(sessionLocale(none, "es-ES"), "es-ES");
  assert.equal(sessionLocale(none, "en_US.UTF-8"), "en-US");
  assert.equal(sessionLocale({ ...none, language: "en" }, "es-ES"), "en-ES");
  assert.equal(sessionLocale({ ...none, region: "MX" }, "es-ES"), "es-MX");
  assert.equal(sessionLocale({ language: "es", region: "MX", hourCycle: "h12" }, "en-US"), "es-MX-u-hc-h12");
  assert.equal(sessionLocale({ ...none, hourCycle: "h23" }, "en-US"), "en-US-u-hc-h23");
  // A region that is not one is the system's.
  assert.deepEqual(effectiveLanguageRegion({ language: "", region: "es" }, "en-GB"), { language: "en", region: "GB" });
});

test("the hour cycle in effect: the region's, or the one chosen", () => {
  assert.equal(regionHourCycle("en", "US"), "h12");
  assert.equal(regionHourCycle("en", "GB"), "h23");
  assert.equal(regionHourCycle("es", "ES"), "h23");
  assert.equal(regionHourCycle("en", "ES"), "h23");
  assert.equal(hourCycleOf("en-US-u-hc-h23"), "h23");
  assert.equal(hourCycleOf("es-ES-u-hc-h12"), "h12");
  assert.equal(hourCycleOf(sessionLocale({ ...none, hourCycle: "h12" }, "es-ES")), "h12");
});

test("the keys and their values, as the session's file writes them", () => {
  assert.equal(PREFERENCE_KEYS.language, "language");
  assert.equal(PREFERENCE_KEYS.region, "region");
  assert.equal(PREFERENCE_KEYS.hourCycle, "hour_cycle");
  assert.equal(preferenceText<"language">(""), "");
  assert.equal(preferenceText<"region">("MX"), "MX");
  assert.equal(preferenceText<"hourCycle">("h23"), "h23");
});

test("Language: Automatic named with the system's language, each language named in itself", () => {
  assert.deepEqual(languageItems(en.languageRegion, "en-US"), [
    { value: "", label: "Automatic (English)" },
    { value: "en", label: "English" },
    { value: "es", label: "Español" },
  ]);
  assert.deepEqual(
    languageItems(es.languageRegion, "es-ES").map((item) => item.label),
    ["Automático (español)", "English", "Español"],
  );
  assert.equal(languageItems(en.languageRegion, "es-ES")[0].label, "Automatic (Spanish)");
  assert.equal(languageItems(es.languageRegion, "en-US")[0].label, "Automático (inglés)");
});

test("Region: Automatic named with the system's region, then the regions by name in the session's language", () => {
  const english = regionItems(en.languageRegion, "es-ES", "");
  assert.deepEqual(english[0], { value: "", label: "Automatic (Spain)" });
  assert.deepEqual(
    english.slice(1).map((item) => item.label),
    [
      "Argentina",
      "Australia",
      "Canada",
      "Chile",
      "Colombia",
      "France",
      "Germany",
      "Ireland",
      "Italy",
      "Mexico",
      "Netherlands",
      "Peru",
      "Portugal",
      "Spain",
      "United Kingdom",
      "United States",
    ],
  );
  const spanish = regionItems(es.languageRegion, "es-ES", "");
  assert.deepEqual(spanish[0], { value: "", label: "Automático (España)" });
  assert.deepEqual(
    spanish.slice(1).map((item) => item.label),
    [
      "Alemania",
      "Argentina",
      "Australia",
      "Canadá",
      "Chile",
      "Colombia",
      "España",
      "Estados Unidos",
      "Francia",
      "Irlanda",
      "Italia",
      "México",
      "Países Bajos",
      "Perú",
      "Portugal",
      "Reino Unido",
    ],
  );
  assert.equal(spanish.find((item) => item.value === "US")?.label, "Estados Unidos");
  // Every region the page lists is named in both languages.
  for (const code of REGIONS) {
    assert.ok(en.languageRegion.regionNames[code], code);
    assert.ok(es.languageRegion.regionNames[code], code);
  }
  // A region chosen elsewhere is offered, by its code where it has no name.
  assert.ok(regionItems(en.languageRegion, "en-US", "JP").some((item) => item.value === "JP" && item.label === "JP"));
  assert.equal(regionItems(en.languageRegion, "ja-JP", "")[0].label, "Automatic (JP)");
});

test("Time format: Automatic named with the region's hour cycle", () => {
  assert.deepEqual(timeFormatItems(en.languageRegion, none, "en-US"), [
    { value: "auto", label: "Automatic (12-hour)" },
    { value: "h12", label: "12-hour" },
    { value: "h23", label: "24-hour" },
  ]);
  assert.deepEqual(
    timeFormatItems(es.languageRegion, none, "es-ES").map((item) => item.label),
    ["Automático (24 horas)", "12 horas", "24 horas"],
  );
  // The region chosen, not the system's.
  assert.equal(timeFormatItems(en.languageRegion, { language: "", region: "GB" }, "en-US")[0].label, "Automatic (24-hour)");
});

/** A formatted text with its spaces plain: ICU puts a narrow or no-break space before "PM" in
    some versions and a space in others. */
function plain(sample: Record<string, string>): Record<string, string> {
  return Object.fromEntries(Object.entries(sample).map(([key, text]) => [key, text.replace(/[\u00a0\u202f]/g, " ")]));
}

test("the sample: a date, a time and a number as the choice formats them", () => {
  assert.deepEqual(plain(localeSample("en-US")), {
    date: "Tuesday, January 5, 2027",
    time: "2:45 PM",
    number: "1,234,567.89",
  });
  assert.deepEqual(plain(localeSample("es-ES")), {
    date: "martes, 5 de enero de 2027",
    time: "14:45",
    number: "1.234.567,89",
  });
  assert.equal(localeSample("en-ES").time, "14:45");
  assert.equal(localeSample("en-US-u-hc-h23").time, "14:45");
  assert.equal(plain(localeSample("es-ES-u-hc-h12")).time, "2:45 p. m.");
  assert.match(localeSample("en-GB").date, /^Tuesday,? 5 January 2027$/);
});

/** The keys of a catalogue, every level down, functions as leaves. */
function shape(value: unknown, path = ""): string[] {
  if (typeof value !== "object" || value === null) return [`${path}:${typeof value}`];
  return Object.keys(value)
    .sort()
    .flatMap((key) => shape((value as Record<string, unknown>)[key], `${path}.${key}`));
}

test("every catalogue has English's shape, and Spanish is not English pasted in", () => {
  assert.deepEqual(shape(CATALOGUES.es), shape(CATALOGUES.en));
  assert.notEqual(CATALOGUES.es.pages.languageRegion, CATALOGUES.en.pages.languageRegion);
  assert.equal(CATALOGUES.en.pages.languageRegion, "Language & Region");
  assert.equal(CATALOGUES.es.pages.languageRegion, "Idioma y región");
  assert.equal(CATALOGUES.en.groups.general, "General");
  assert.equal(CATALOGUES.es.groups.general, "General");
  assert.equal(messagesLanguage("es-MX-u-hc-h12"), "es");
  assert.equal(messagesLanguage(undefined), "en");
  assert.equal(messagesFor("es"), CATALOGUES.es);
});

test("counts through Intl.PluralRules, worded as before", () => {
  assert.equal(en.lockScreen.minutes(1), "1 minute");
  assert.equal(en.lockScreen.minutes(5), "5 minutes");
  assert.equal(en.lockScreen.minutes(60), "1 hour");
  assert.equal(es.lockScreen.minutes(1), "1 minuto");
  assert.equal(es.lockScreen.minutes(30), "30 minutos");
  assert.equal(es.lockScreen.minutes(60), "1 hora");
  assert.equal(en.displays.keepDescription(15), "The display goes back to its previous settings in 15 seconds.");
  assert.equal(es.displays.keepDescription(1), "La pantalla vuelve a su configuración anterior en 1 segundo.");
  assert.equal(es.displays.keepDescription(0), "La pantalla vuelve a su configuración anterior en 0 segundos.");
});
