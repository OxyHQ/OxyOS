/**
 * The shell's dates and times (docs/adr/0040-language-and-region.mdx, decision 5): `Intl` in the
 * session's locale, whose hour cycle (the region's, or the person's `-u-hc-h12`/`-u-hc-h23`) says
 * whether the clock counts 12 or 24 hours. A 24-hour clock shows two-digit hours ("09:41"), a
 * 12-hour one the hour as it is said ("9:41 AM"), as a Mac's menu bar does.
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

function timeFormat(locale: string): Intl.DateTimeFormatOptions {
  return twelveHour(locale) ? { hour: "numeric", minute: "2-digit" } : { hour: "2-digit", minute: "2-digit" };
}

/** The time as a clock shows it: "09:41", "9:41 AM". */
export function clockTime(now: Date, locale: string): string {
  return format(locale, timeFormat(locale)).format(now);
}

/** The top bar's clock: the short weekday and date, then the time ("lun, 2 nov 09:41",
    "Mon, Nov 2 9:41 AM"). */
export function barClock(now: Date, locale: string): string {
  const date = format(locale, { weekday: "short", day: "numeric", month: "short" }).format(now);
  return `${date} ${clockTime(now, locale)}`;
}

/** The lock screen's date, its first letter capitalized ("Lunes, 2 de noviembre",
    "Monday, November 2"). */
export function lockDate(now: Date, locale: string): string {
  const date = format(locale, { weekday: "long", day: "numeric", month: "long" }).format(now);
  return date.charAt(0).toUpperCase() + date.slice(1);
}

/**
 * The lock screen's time: `shown` large, without the day period, as a Mac's lock screen shows
 * "9:41"; `said`, the whole time, for a screen reader ("9:41 AM").
 */
export function lockTime(now: Date, locale: string): { shown: string; said: string } {
  const time = format(locale, timeFormat(locale));
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
