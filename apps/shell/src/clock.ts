/**
 * The shell's dates and times (docs/adr/0040-language-and-region.mdx, decision 5): `Intl` in the
 * session's locale, whose hour cycle (the region's, or the person's `-u-hc-h12`/`-u-hc-h23`) says
 * whether the clock counts 12 or 24 hours. The time is the locale's own short time (CLDR's), as a
 * Mac's menu bar shows it: the hour padded where the locale pads it ("09:41" in en-GB and, since
 * CLDR 48, in es-ES; "9:41" in ja-JP; "9:41 AM" in en-US), never padded or unpadded by the shell.
 */

const formats = new Map<string, Intl.DateTimeFormat>();

function format(locale: string, options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = `${locale}|${JSON.stringify(options)}`;
  let made = formats.get(key);
  if (!made) {
    made = new Intl.DateTimeFormat(locale, options);
    formats.set(key, made);
  }
  return made;
}

/** Whether the locale's clock counts 12 hours (its hour cycle `h11` or `h12`). */
export function twelveHour(locale: string): boolean {
  const cycle = format(locale, { hour: "numeric" }).resolvedOptions().hourCycle;
  return cycle === "h11" || cycle === "h12";
}

/** The locale's short time (`timeStyle: "short"`): its own pattern and hour cycle. */
const SHORT_TIME: Intl.DateTimeFormatOptions = { timeStyle: "short" };

/** The time as a clock shows it, in the locale's short time: "09:41" (en-GB), "9:41 AM" (en-US). */
export function clockTime(now: Date, locale: string): string {
  return format(locale, SHORT_TIME).format(now);
}

/** The top bar's clock: the short weekday and date, then the time ("lun, 2 nov 09:41",
    "Mon, Nov 2 9:41 AM"). */
export function barClock(now: Date, locale: string): string {
  const date = format(locale, { weekday: "short", day: "numeric", month: "short" }).format(now);
  return `${date} ${clockTime(now, locale)}`;
}

/** The top bar's clock as a screen reader says it: the same date and time in whole words, as
    VoiceOver names a Mac's menu bar clock by what it shows ("martes, 3 de noviembre, 14:05",
    "Tuesday, November 3, 2:05 PM"). */
export function clockSaid(now: Date, locale: string): string {
  const date = format(locale, { weekday: "long", day: "numeric", month: "long" }).format(now);
  return `${date}, ${clockTime(now, locale)}`;
}

/** The lock screen's date, cased as the locale writes it, as a Mac's lock screen shows it: names
    of days and months stay lower case where the language writes them so ("lunes, 2 de noviembre",
    "Monday, November 2"). */
export function lockDate(now: Date, locale: string): string {
  return format(locale, { weekday: "long", day: "numeric", month: "long" }).format(now);
}

/**
 * The lock screen's time: `shown` large, without the day period, as a Mac's lock screen shows
 * "9:41"; `said`, the whole time, for a screen reader ("9:41 AM").
 */
export function lockTime(now: Date, locale: string): { shown: string; said: string } {
  const time = format(locale, SHORT_TIME);
  const said = time.format(now);
  if (!twelveHour(locale)) return { shown: said, said };
  // The hour, the minutes and what is between them, whichever side the day period is on.
  const parts = time.formatToParts(now);
  const digits = (part: Intl.DateTimeFormatPart) => part.type === "hour" || part.type === "minute";
  const first = parts.findIndex(digits);
  const last = parts.length - 1 - [...parts].reverse().findIndex(digits);
  const shown = first < 0 ? said : parts.slice(first, last + 1).map((part) => part.value).join("");
  return { shown, said };
}
