// ECMA-402's Intl on OxyOS (intl.h): DateTimeFormat and NumberFormat, getCanonicalLocales, and the
// Date and Number toLocale*String methods built on them, as Hermes provides them on Android. This
// layer reads the options as ECMA-402 does (GetOption, ToDateTimeOptions); the formatting is the
// host's (`__oxyosIntl`, crates/oxyos-intl on ICU4X), which refuses with a RangeError what it does
// not support yet. Intl's other constructors are left undefined, so feature detection finds them
// absent rather than present and wrong. Evaluated before the application's bundle.
(function (native) {
  'use strict';
  const define = (target, name, value) =>
    Object.defineProperty(target, name, { value, writable: true, configurable: true, enumerable: false });

  // CanonicalizeLocaleList: a tag or a list of them, each canonicalized, without repeats.
  function canonicalLocales(locales) {
    if (locales === undefined) return [];
    const list = typeof locales === 'string' ? [locales] : Array.from(Object(locales));
    const seen = [];
    for (const tag of list) {
      if (typeof tag !== 'string' && (typeof tag !== 'object' || tag === null)) {
        throw new TypeError('A locale must be a string');
      }
      const canonical = native.canonicalize(String(tag));
      if (!seen.includes(canonical)) seen.push(canonical);
    }
    return seen;
  }

  // GetOption: the option converted to its type, one of `values` if given, else the fallback.
  function option(options, name, type, values, fallback) {
    let value = options[name];
    if (value === undefined) return fallback;
    if (type === 'boolean') value = Boolean(value);
    if (type === 'string') value = String(value);
    if (type === 'number') {
      value = Number(value);
      if (!Number.isFinite(value)) throw new RangeError(`${name} must be a finite number`);
    }
    if (values && !values.includes(value)) throw new RangeError(`Value ${value} out of range for ${name}`);
    return value;
  }

  // The options as the host takes them: "name=value" pairs joined by ';'.
  const serialize = (options) =>
    Object.keys(options)
      .filter((name) => options[name] !== undefined)
      .map((name) => `${name}=${options[name]}`)
      .join(';');

  const parse = (pairs) => {
    const out = {};
    for (const pair of pairs ? pairs.split(';') : []) {
      const at = pair.indexOf('=');
      const name = pair.slice(0, at);
      const value = pair.slice(at + 1);
      out[name] = value === 'true' ? true : value === 'false' ? false : /^-?\d+$/.test(value) ? Number(value) : value;
    }
    return out;
  };

  const DATE_OPTIONS = {
    weekday: ['narrow', 'short', 'long'],
    era: ['narrow', 'short', 'long'],
    year: ['2-digit', 'numeric'],
    month: ['2-digit', 'numeric', 'narrow', 'short', 'long'],
    day: ['2-digit', 'numeric'],
    dayPeriod: ['narrow', 'short', 'long'],
    hour: ['2-digit', 'numeric'],
    minute: ['2-digit', 'numeric'],
    second: ['2-digit', 'numeric'],
    timeZoneName: ['short', 'long', 'shortOffset', 'longOffset', 'shortGeneric', 'longGeneric'],
  };
  const STYLES = ['full', 'long', 'medium', 'short'];

  // ToDateTimeOptions: the date or time fields a method needs, numeric, when none is asked for.
  function dateTimeOptions(options, required, defaults) {
    const out = options === undefined ? {} : Object.assign(Object.create(null), Object(options));
    let needDefaults = true;
    if (required === 'date' || required === 'any') {
      for (const name of ['weekday', 'year', 'month', 'day']) if (out[name] !== undefined) needDefaults = false;
    }
    if (required === 'time' || required === 'any') {
      for (const name of ['dayPeriod', 'hour', 'minute', 'second', 'fractionalSecondDigits']) {
        if (out[name] !== undefined) needDefaults = false;
      }
    }
    if (out.dateStyle !== undefined || out.timeStyle !== undefined) needDefaults = false;
    if (required === 'date' && out.timeStyle !== undefined) throw new TypeError('timeStyle cannot be used here');
    if (required === 'time' && out.dateStyle !== undefined) throw new TypeError('dateStyle cannot be used here');
    if (needDefaults && (defaults === 'date' || defaults === 'all')) {
      out.year = out.month = out.day = 'numeric';
    }
    if (needDefaults && (defaults === 'time' || defaults === 'all')) {
      out.hour = out.minute = out.second = 'numeric';
    }
    return out;
  }

  const internal = new WeakMap();
  const slots = (object, kind) => {
    const found = internal.get(object);
    if (!found || found.kind !== kind) throw new TypeError(`Method called on an object that is not an Intl.${kind}`);
    return found;
  };

  // A time value from a Date or a number; now when absent.
  function timeValue(date) {
    const value = date === undefined ? Date.now() : Number(date);
    if (!Number.isFinite(value)) throw new RangeError('Invalid time value');
    return value;
  }

  const toParts = (parts) => parts.map(([type, value]) => ({ type, value }));

  function DateTimeFormat(locales, options) {
    if (!new.target) return new DateTimeFormat(locales, options);
    const requested = canonicalLocales(locales);
    const given = dateTimeOptions(options, 'any', 'date');
    const chosen = {};
    for (const name of Object.keys(DATE_OPTIONS)) {
      chosen[name] = option(given, name, 'string', DATE_OPTIONS[name], undefined);
    }
    chosen.fractionalSecondDigits = option(given, 'fractionalSecondDigits', 'number', [1, 2, 3], undefined);
    chosen.hour12 = option(given, 'hour12', 'boolean', undefined, undefined);
    chosen.hourCycle = option(given, 'hourCycle', 'string', ['h11', 'h12', 'h23', 'h24'], undefined);
    chosen.timeZone = option(given, 'timeZone', 'string', undefined, undefined);
    chosen.dateStyle = option(given, 'dateStyle', 'string', STYLES, undefined);
    chosen.timeStyle = option(given, 'timeStyle', 'string', STYLES, undefined);
    chosen.calendar = option(given, 'calendar', 'string', undefined, undefined);
    chosen.numberingSystem = option(given, 'numberingSystem', 'string', undefined, undefined);
    option(given, 'localeMatcher', 'string', ['lookup', 'best fit'], 'best fit');
    option(given, 'formatMatcher', 'string', ['basic', 'best fit'], 'best fit');
    if ((chosen.dateStyle || chosen.timeStyle) && Object.keys(DATE_OPTIONS).some((name) => chosen[name])) {
      throw new TypeError('dateStyle and timeStyle cannot be combined with other date and time options');
    }
    const locale = native.resolve(requested.join(','));
    const serialized = serialize(chosen);
    // The host checks the options now, as the constructor would: a refusal throws here.
    const resolved = parse(native.resolveDate(locale, serialized));
    internal.set(this, { kind: 'DateTimeFormat', locale, options: serialized, resolved });
  }
  Object.defineProperty(DateTimeFormat.prototype, 'format', {
    configurable: true,
    get() {
      const self = slots(this, 'DateTimeFormat');
      if (!self.bound) {
        self.bound = (date) =>
          native
            .formatDate(self.locale, self.options, timeValue(date))
            .map(([, value]) => value)
            .join('');
      }
      return self.bound;
    },
  });
  define(DateTimeFormat.prototype, 'formatToParts', function formatToParts(date) {
    const self = slots(this, 'DateTimeFormat');
    return toParts(native.formatDate(self.locale, self.options, timeValue(date)));
  });
  define(DateTimeFormat.prototype, 'resolvedOptions', function resolvedOptions() {
    return Object.assign({}, slots(this, 'DateTimeFormat').resolved);
  });
  // The requested locales there is data for: those the lookup matcher resolves to their own
  // language rather than falling back to the default.
  define(DateTimeFormat, 'supportedLocalesOf', function supportedLocalesOf(locales) {
    const language = (tag) => tag.split('-')[0];
    return canonicalLocales(locales).filter((tag) => language(native.resolve(tag)) === language(tag));
  });

  const STYLE = ['decimal', 'percent', 'currency', 'unit'];
  function NumberFormat(locales, options) {
    if (!new.target) return new NumberFormat(locales, options);
    const requested = canonicalLocales(locales);
    const given = options === undefined ? {} : Object(options);
    const chosen = {
      style: option(given, 'style', 'string', STYLE, undefined),
      currency: option(given, 'currency', 'string', undefined, undefined),
      currencyDisplay: option(given, 'currencyDisplay', 'string', ['code', 'symbol', 'narrowSymbol', 'name'], undefined),
      currencySign: option(given, 'currencySign', 'string', ['standard', 'accounting'], undefined),
      unit: option(given, 'unit', 'string', undefined, undefined),
      unitDisplay: option(given, 'unitDisplay', 'string', ['short', 'narrow', 'long'], undefined),
      notation: option(given, 'notation', 'string', ['standard', 'scientific', 'engineering', 'compact'], undefined),
      compactDisplay: option(given, 'compactDisplay', 'string', ['short', 'long'], undefined),
      minimumIntegerDigits: option(given, 'minimumIntegerDigits', 'number', undefined, undefined),
      minimumFractionDigits: option(given, 'minimumFractionDigits', 'number', undefined, undefined),
      maximumFractionDigits: option(given, 'maximumFractionDigits', 'number', undefined, undefined),
      minimumSignificantDigits: option(given, 'minimumSignificantDigits', 'number', undefined, undefined),
      maximumSignificantDigits: option(given, 'maximumSignificantDigits', 'number', undefined, undefined),
      useGrouping: option(given, 'useGrouping', 'string', ['always', 'auto', 'min2', 'true', 'false'], undefined),
      signDisplay: option(given, 'signDisplay', 'string', ['auto', 'never', 'always', 'exceptZero', 'negative'], undefined),
      numberingSystem: option(given, 'numberingSystem', 'string', undefined, undefined),
    };
    if (chosen.style === 'currency' && chosen.currency === undefined) {
      throw new TypeError('Currency code is required with currency style');
    }
    option(given, 'localeMatcher', 'string', ['lookup', 'best fit'], 'best fit');
    const locale = native.resolve(requested.join(','));
    const serialized = serialize(chosen);
    const resolved = parse(native.resolveNumber(locale, serialized));
    internal.set(this, { kind: 'NumberFormat', locale, options: serialized, resolved });
  }
  Object.defineProperty(NumberFormat.prototype, 'format', {
    configurable: true,
    get() {
      const self = slots(this, 'NumberFormat');
      if (!self.bound) {
        self.bound = (value) =>
          native
            .formatNumber(self.locale, self.options, Number(value))
            .map(([, text]) => text)
            .join('');
      }
      return self.bound;
    },
  });
  define(NumberFormat.prototype, 'formatToParts', function formatToParts(value) {
    const self = slots(this, 'NumberFormat');
    return toParts(native.formatNumber(self.locale, self.options, Number(value)));
  });
  define(NumberFormat.prototype, 'resolvedOptions', function resolvedOptions() {
    return Object.assign({}, slots(this, 'NumberFormat').resolved);
  });
  define(NumberFormat, 'supportedLocalesOf', DateTimeFormat.supportedLocalesOf);

  const Intl = {};
  define(Intl, 'DateTimeFormat', DateTimeFormat);
  define(Intl, 'NumberFormat', NumberFormat);
  define(Intl, 'getCanonicalLocales', function getCanonicalLocales(locales) {
    return canonicalLocales(locales);
  });
  Object.defineProperty(Intl, Symbol.toStringTag, { value: 'Intl', configurable: true });
  define(globalThis, 'Intl', Intl);

  // The toLocale*String methods, as ECMA-402 defines them over DateTimeFormat and NumberFormat.
  const localeString = (required, defaults) =>
    function (locales, options) {
      const value = Number(this);
      if (!Number.isFinite(value)) return 'Invalid Date';
      return new DateTimeFormat(locales, dateTimeOptions(options, required, defaults)).format(value);
    };
  define(Date.prototype, 'toLocaleString', localeString('any', 'all'));
  define(Date.prototype, 'toLocaleDateString', localeString('date', 'date'));
  define(Date.prototype, 'toLocaleTimeString', localeString('time', 'time'));
  define(Number.prototype, 'toLocaleString', function toLocaleString(locales, options) {
    return new NumberFormat(locales, options).format(Number(this));
  });
})(globalThis.__oxyosIntl);
