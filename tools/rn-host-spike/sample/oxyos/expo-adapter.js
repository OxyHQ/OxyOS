/**
 * OxyOS side of the Expo modules Bloom and Oxy apps reach (expo-font, expo-asset beneath it,
 * expo-clipboard, expo-notifications, expo-constants, expo-crypto, expo-linking,
 * expo-localization, expo-secure-store, expo-haptics, expo-web-browser, @expo/app-integrity,
 * expo-blur's views and expo-image's view and module). expo-modules-core's own global polyfill
 * provides the module classes; the native work is done by the host's TurboModules
 * (OxyOSFontRegistry, OxyOSClipboard, OxyOSNotifications, OxyOSApplication, OxyOSCrypto,
 * OxyOSLocale, OxyOSExpoImage) and host components (expo-blur.h, expo-image.h). Anything OxyOS
 * cannot do fails loudly.
 * expo-file-system's module is expo-file-system.js, installed after this.
 */
import { launch, requestCapability } from './system';
import { installExpoGlobalPolyfill } from 'expo-modules-core/src/polyfill/dangerous-internal';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

installExpoGlobalPolyfill();

// The kernel's CSPRNG and digests (crates/oxyos-client-ffi): expo-crypto's, and Expo's own
// version 4 UUIDs (expo-notifications names a notification by one), as its native side makes them.
const crypto = TurboModuleRegistry.getEnforcing('OxyOSCrypto');
const uuidv4 = () => {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
};
globalThis.expo.uuidv4 = uuidv4;

// expo-crypto, as its Android module: getRandomValues fills an integer typed array in place,
// digests are SHA-1, SHA-256, SHA-384 and SHA-512 (MD2, MD4 and MD5 throw, naming the
// algorithm).
globalThis.expo.modules.ExpoCrypto = {
  getRandomValues: (typedArray) => crypto.getRandomValues(typedArray),
  randomUUID: uuidv4,
  digestString: (algorithm, data, options) => crypto.digestString(algorithm, String(data), options?.encoding ?? 'hex'),
  digestStringAsync: async (algorithm, data, options) =>
    crypto.digestString(algorithm, String(data), options?.encoding ?? 'hex'),
  digest: (algorithm, output, data) => crypto.digest(algorithm, output, data),
};

// expo-crypto's AES-GCM (its `aes` module, which the package loads with the rest): not provided on
// OxyOS yet. Its classes and functions are there so that the rest of expo-crypto loads; making a
// key or sealed data, encrypting or decrypting fails, naming it.
const noAes = "expo-crypto's AES is not available on OxyOS yet";
class EncryptionKey {
  constructor() {
    throw new Error(noAes);
  }
  static generate = async () => {
    throw new Error(noAes);
  };
  static import = async () => {
    throw new Error(noAes);
  };
}
class SealedData {
  constructor() {
    throw new Error(noAes);
  }
  static fromParts = () => {
    throw new Error(noAes);
  };
  static fromCombined = () => {
    throw new Error(noAes);
  };
}
globalThis.expo.modules.ExpoCryptoAES = {
  EncryptionKey,
  SealedData,
  encryptAsync: async () => {
    throw new Error(noAes);
  },
  decryptAsync: async () => {
    throw new Error(noAes);
  },
};

// expo-constants: the application this process runs (OxyOSApplication), as its Android
// ConstantsService answers: the public app config its build embedded next to the bundle
// (`manifest`, null when it has none, as Android without `app.config`), a bare build's execution
// environment, no status bar, the computer's name and the system's version. Nothing of a
// platform's own (`platform` names none of Android's or iOS's).
const application = TurboModuleRegistry.getEnforcing('OxyOSApplication').getConstants();
globalThis.expo.modules.ExponentConstants = {
  sessionId: uuidv4(),
  executionEnvironment: 'bare',
  statusBarHeight: 0,
  deviceName: application.deviceName || null,
  systemFonts: [],
  systemVersion: application.systemVersion || null,
  manifest: application.manifest,
  platform: {},
  getWebViewUserAgentAsync: async () => null,
};

// expo-linking, as its Android module (and React Native's Linking, the host's IntentAndroid): the
// linking URL is what the session started the application to open (its launch record's: a file,
// or a URL of a scheme its entry declares, which Expo Router opens at its route,
// docs/adr/0034-the-dock.mdx), as an Android intent's data; none for a plain launch. None arrives
// later (`onURLReceived` is never sent): a URL for an application running starts another process.
let linkingURL = application.launchURL ?? null;
const ExpoLinking = new globalThis.expo.EventEmitter();
Object.assign(ExpoLinking, {
  getLinkingURL: () => linkingURL,
  clearInitialURL: () => {
    linkingURL = null;
  },
});
globalThis.expo.modules.ExpoLinking = ExpoLinking;

// expo-haptics: OxyOS drives no haptic hardware, so feedback resolves without effect, as Expo's
// own implementation does on a desktop that has none (its web module, with no vibration and no
// coarse pointer). Android's performHapticsAsync is Android's alone and is not provided.
globalThis.expo.modules.ExpoHaptics = {
  impactAsync: async () => {},
  notificationAsync: async () => {},
  selectionAsync: async () => {},
};

// @expo/app-integrity: App Attest and Play Integrity are iOS's and Android's, and the package's own
// functions say so on any other platform (its web module is empty too); OxyOS cannot yet prove an
// application's identity to a server. The module is there so that importing the package does not
// fail before those functions can say it.
globalThis.expo.modules.ExpoAppIntegrity = {};

// expo-web-browser: OxyOS has no web browser yet, so opening a page or an authentication session
// rejects with that reason (the Oxy SDK's sign-in then tries Linking.openURL, which rejects too).
// With nothing open, dismissing leaves nothing open.
const noBrowser = async () => {
  throw new Error('OxyOS has no web browser yet');
};
globalThis.expo.modules.ExpoWebBrowser = {
  openBrowserAsync: noBrowser,
  openAuthSessionAsync: noBrowser,
  warmUpAsync: noBrowser,
  mayInitWithUrlAsync: noBrowser,
  coolDownAsync: noBrowser,
  getCustomTabsSupportingBrowsersAsync: noBrowser,
  dismissBrowser: async () => ({ type: 'dismiss' }),
  dismissAuthSession: () => {},
};

// expo-secure-store: OxyOS keeps no secrets for an application. The machine's Oxy account is the
// platform's (docs/adr/0020-oxy-account-login.mdx, decision 6): an application never holds the
// device credential, and the Oxy SDK in it takes the session from the platform (the session host
// the framework registers, session-host.js), never from storage of its own. So nothing is ever
// kept: a read finds nothing, a delete leaves nothing, and keeping a value fails with the reason.
// The module is there, rather than missing, so that what an application keeps in it never falls
// back to plain storage (the Oxy SDK's own fallback, AsyncStorage) instead.
const noSecrets =
  'OxyOS keeps no secrets for applications: the Oxy session is the platform\'s (ADR 0020), and the Oxy SDK takes it from the platform';
let toldNoSecrets = false;
const nothingKept = () => {
  if (!toldNoSecrets) {
    toldNoSecrets = true;
    console.warn(`expo-secure-store: ${noSecrets}. Nothing is kept here.`);
  }
  return null;
};
globalThis.expo.modules.ExpoSecureStore = {
  AFTER_FIRST_UNLOCK: 0,
  AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY: 1,
  ALWAYS: 2,
  WHEN_PASSCODE_SET_THIS_DEVICE_ONLY: 3,
  ALWAYS_THIS_DEVICE_ONLY: 4,
  WHEN_UNLOCKED: 5,
  WHEN_UNLOCKED_THIS_DEVICE_ONLY: 6,
  getValueWithKeyAsync: async () => nothingKept(),
  getValueWithKeySync: () => nothingKept(),
  deleteValueWithKeyAsync: async () => {},
  setValueWithKeyAsync: async () => {
    throw new Error(noSecrets);
  },
  setValueWithKeySync: () => {
    throw new Error(noSecrets);
  },
  canUseBiometricAuthentication: () => false,
};

// expo-localization (docs/adr/0040-language-and-region.mdx, decision 4), as its Android module
// answers: one locale and one calendar, the session's (the host's OxyOSLocale: the tag the session
// last told, `es-MX-u-hc-h12`, else the environment's), read at each call; on a change, both of its
// events, as Android's configuration change sends them, so useLocales() and useCalendars() render
// again. Separators are the host's Intl's (CLDR's, through ICU4X); the measurement system, the
// temperature unit and the first day of the week are CLDR's by region (below). No currency: OxyOS
// has no region-to-currency data yet, and null says so rather than a guess.
const localeModule = TurboModuleRegistry.get('OxyOSLocale');
const sessionLocale = () => localeModule?.getLocale() || Intl.DateTimeFormat().resolvedOptions().locale;
// A tag's language, script and region: those before its first singleton (`-u-`, `-x-`).
const subtagsOf = (tag) => {
  const found = { language: null, script: null, region: null };
  const subtags = tag.split('-');
  found.language = subtags[0] || null;
  for (const subtag of subtags.slice(1)) {
    if (subtag.length === 1) break;
    if (/^[A-Za-z]{4}$/.test(subtag) && !found.script && !found.region) found.script = subtag;
    else if (/^([A-Za-z]{2}|\d{3})$/.test(subtag) && !found.region) found.region = subtag.toUpperCase();
  }
  return found;
};
// CLDR's measurementData (supplementalData.xml): `US` for the United States, Liberia and Myanmar,
// `UK` for the United Kingdom; metric (001) elsewhere.
const measurementOf = (region) => (['US', 'LR', 'MM'].includes(region) ? 'us' : region === 'GB' ? 'uk' : 'metric');
// Fahrenheit where the region's temperature is told in it (expo-localization's Android table).
const temperatureOf = (region) =>
  region === null ? null : ['US', 'BS', 'BZ', 'KY', 'PW', 'LR'].includes(region) ? 'fahrenheit' : 'celsius';
// CLDR 48's weekData firstDay (as ICU 78.3 gives it, Intl.Locale's getWeekInfo), the regions whose
// week does not begin on Monday, CLDR's 001; in expo-localization's numbering, 1 Sunday to 7 Saturday.
const FIRST_DAYS = {
  1: 'AG AS BD BR BS BT BW BZ CA CO DM DO ET GT GU HK HN ID IL IN IS JM JP KE KH KR LA MH MM MO MT MX MZ NI NP PA PE PH PK PR PT PY SA SG SV TH TT TW UM US VE VI WS YE ZA ZW',
  6: 'MV',
  7: 'AF BH DJ DZ EG IQ IR JO KW LY OM QA SD SY',
};
const firstWeekdayOf = (region) =>
  Number(Object.keys(FIRST_DAYS).find((day) => FIRST_DAYS[day].split(' ').includes(region)) ?? 2);
const separatorsOf = (tag) => {
  const parts = new Intl.NumberFormat(tag).formatToParts(1234567.5);
  return {
    decimal: parts.find((part) => part.type === 'decimal')?.value ?? null,
    group: parts.find((part) => part.type === 'group')?.value ?? null,
  };
};
const ExpoLocalization = new globalThis.expo.EventEmitter();
Object.assign(ExpoLocalization, {
  getLocales: () => {
    const tag = sessionLocale();
    const { language, script, region } = subtagsOf(tag);
    const separators = separatorsOf(tag);
    return [
      {
        // Without its extensions, as Android's system locale has none (the hour cycle is
        // getCalendars()' uses24hourClock).
        languageTag: [language, script, region].filter(Boolean).join('-'),
        languageCode: language,
        languageScriptCode: script,
        regionCode: region,
        languageRegionCode: region,
        currencyCode: null,
        currencySymbol: null,
        languageCurrencyCode: null,
        languageCurrencySymbol: null,
        decimalSeparator: separators.decimal,
        digitGroupingSeparator: separators.group,
        // Both of OxyOS's languages are written left to right.
        textDirection: 'ltr',
        measurementSystem: measurementOf(region),
        temperatureUnit: temperatureOf(region),
      },
    ];
  },
  getCalendars: () => {
    const tag = sessionLocale();
    const { hourCycle } = new Intl.DateTimeFormat(tag, { hour: 'numeric' }).resolvedOptions();
    return [
      {
        calendar: 'gregory',
        uses24hourClock: hourCycle === 'h23' || hourCycle === 'h24',
        firstWeekday: firstWeekdayOf(subtagsOf(tag).region),
        timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone ?? null,
      },
    ];
  },
});
DeviceEventEmitter.addListener('OxyOSLocaleChanged', () => {
  ExpoLocalization.emit('onLocaleSettingsChanged');
  ExpoLocalization.emit('onCalendarSettingsChanged');
});
globalThis.expo.modules.ExpoLocalization = ExpoLocalization;

const fonts = TurboModuleRegistry.getEnforcing('OxyOSFontRegistry');
const loaded = new Set();

globalThis.expo.modules.ExpoFontLoader = {
  getLoadedFonts: () => [...loaded],
  isLoaded: (family) => loaded.has(family),
  loadAsync: async (family, uri) => {
    if (!fonts.registerFont(family, String(uri))) {
      throw new Error(`OxyOS could not load font "${family}" from ${uri}`);
    }
    loaded.add(family);
  },
};

globalThis.expo.modules.ExpoAsset = {
  // Bundled assets are already files next to the bundle; OxyOS has no asset download service.
  downloadAsync: async (url) => {
    if (String(url).startsWith('file://')) return url;
    throw new Error(`OxyOS cannot download asset ${url}`);
  },
};

// expo-clipboard, on the OS clipboard (docs/adr/0007-clipboard.mdx). Reading and writing it use
// the serial of the latest key or press the app received, so the compositor refuses both unless
// the app has keyboard focus and acted just before: a read refused that way gives "".
const clipboard = TurboModuleRegistry.getEnforcing('OxyOSClipboard');
const isUrl = (text) => /^[a-z][a-z0-9+.-]*:\/\/\S+$/i.test(text.trim());
const noImages = async () => {
  throw new Error('Images on the clipboard are not supported on OxyOS yet');
};
const ExpoClipboard = new globalThis.expo.EventEmitter();
Object.assign(ExpoClipboard, {
  // Plain text only: HTML is offered as no other type on OxyOS yet.
  getStringAsync: () => clipboard.getString(),
  setStringAsync: (text) => clipboard.setString(String(text)),
  hasStringAsync: () => clipboard.hasString(),
  getUrlAsync: async () => {
    const text = await clipboard.getString();
    return isUrl(text) ? text.trim() : null;
  },
  setUrlAsync: (url) => clipboard.setString(String(url)),
  hasUrlAsync: async () => isUrl(await clipboard.getString()),
  getImageAsync: noImages,
  setImageAsync: noImages,
  hasImageAsync: noImages,
});
DeviceEventEmitter.addListener('OxyOSClipboardChanged', ({ text }) =>
  ExpoClipboard.emit('onClipboardChanged', { contentTypes: text ? ['plain-text'] : [] }),
);
globalThis.expo.modules.ExpoClipboard = ExpoClipboard;

// Expo's native views the host draws (expo-blur.h, expo-image.h), by module and view, with the
// props each takes and the events it sends as its Android view definition names them; React
// Native's view props come with every view.
const expoViews = {
  'ExpoBlur.ExpoBlurView': {
    props: ['blurTargetId', 'intensity', 'tint', 'blurReductionFactor', 'borderRadii', 'blurMethod'],
  },
  'ExpoBlur.ExpoBlurTargetView': { props: [] },
  ExpoImage: {
    props: [
      'source', 'placeholder', 'contentFit', 'placeholderContentFit', 'contentPosition', 'transition', 'tintColor',
      'blurRadius', 'cachePolicy', 'priority', 'recyclingKey', 'autoplay', 'allowDownscaling', 'accessible',
      'accessibilityLabel', 'focusable', 'decodeFormat', 'svgVariables',
    ],
    events: ['onLoadStart', 'onProgress', 'onError', 'onLoad', 'onDisplay'],
  },
};
globalThis.expo.getViewConfig = (moduleName, viewName) => {
  const view = expoViews[viewName ? `${moduleName}.${viewName}` : moduleName];
  if (!view) throw new Error(`Expo's ${moduleName} ${viewName ?? 'view'} is not available on OxyOS`);
  return {
    validAttributes: Object.fromEntries(view.props.map((prop) => [prop, true])),
    // As Expo's native side names them: `onLoad` is the event `topLoad`.
    directEventTypes: Object.fromEntries(
      (view.events ?? []).map((name) => [`top${name.slice(2)}`, { registrationName: name }]),
    ),
  };
};

// expo-image's module (images.h, through the host's OxyOSExpoImage), as its Android module:
// Image.prefetch into the caches its cachePolicy names, clearMemoryCache, clearDiskCache,
// getCachePathAsync, loadAsync's ImageRef (useImage's too), and its view's functions. Writing to
// or reading an ImageRef from the cache and generating hashes are not available on OxyOS.
const expoImages = TurboModuleRegistry.get('OxyOSExpoImage');
const noExpoImage = (what) => async () => {
  throw new Error(`expo-image's ${what} is not available on OxyOS`);
};
// An ImageRef: a picture the host holds by its id (`__expo_shared_object_id__`, what a view's
// `source` sends), released by release() or when JavaScript collects it (its native handle).
const imageHandle = Symbol('handle');
class ExpoImageRef extends globalThis.expo.SharedRef {
  nativeRefType = 'image';
  release() {
    const id = this.__expo_shared_object_id__;
    if (typeof id === 'number') expoImages?.release(id);
  }
}
const imageRef = (loaded) => {
  const ref = new ExpoImageRef();
  Object.defineProperties(ref, {
    __expo_shared_object_id__: { value: loaded.id },
    [imageHandle]: { value: expoImages.refHandle(loaded) },
    width: { value: loaded.width, enumerable: true },
    height: { value: loaded.height, enumerable: true },
    scale: { value: loaded.scale, enumerable: true },
    isAnimated: { value: loaded.isAnimated, enumerable: true },
    mediaType: { value: null, enumerable: true },
  });
  return ref;
};
const ExpoImage = new globalThis.expo.NativeModule();
Object.assign(ExpoImage, {
  Image: ExpoImageRef,
  prefetch: async (urls, cachePolicy, headers) => {
    if (!expoImages) return false;
    return expoImages.prefetch(urls, cachePolicy ?? 'memory-disk', headers ?? null);
  },
  clearMemoryCache: async () => (expoImages ? expoImages.clearMemoryCache() : false),
  clearDiskCache: async () => (expoImages ? expoImages.clearDiskCache() : false),
  getCachePathAsync: async (cacheKey) => (expoImages ? expoImages.getCachePathAsync(String(cacheKey)) : null),
  loadAsync: async (source, options) => {
    if (!expoImages) throw new Error('expo-image needs a window on OxyOS');
    return imageRef(await expoImages.loadAsync(source, options ?? {}));
  },
  writeToCacheAsync: noExpoImage('writeToCacheAsync'),
  readFromCacheAsync: noExpoImage('readFromCacheAsync'),
  generateBlurhashAsync: noExpoImage('generateBlurhashAsync'),
  generateThumbhashAsync: noExpoImage('generateThumbhashAsync'),
  // iOS's alone.
  configureCache: () => {},
  // The view's functions, called on its component with the view's tag (`nativeTag`).
  ViewPrototypes: {
    ExpoImage: {
      startAnimating() {
        return expoImages?.startAnimating(this.nativeTag);
      },
      stopAnimating() {
        return expoImages?.stopAnimating(this.nativeTag);
      },
      lockResourceAsync() {
        return expoImages?.lockResource(this.nativeTag);
      },
      unlockResourceAsync() {
        return expoImages?.unlockResource(this.nativeTag);
      },
      reloadAsync() {
        return expoImages?.reload(this.nativeTag);
      },
    },
  },
});
globalThis.expo.modules.ExpoImage = ExpoImage;

// expo-notifications (docs/adr/0026-notifications.mdx): an application's notifications are the
// session's, posted through the host's OxyOSNotifications module. The session keeps them (the
// Notification Center), shows them as banners as its preferences allow (Do Not Disturb) and gives
// the application the response once when the person presses one; the application is its launch
// record, never a claim. Only notifications presented now are OxyOS's: a scheduled trigger, push
// tokens, badges and background tasks fail loudly.
const notifications = TurboModuleRegistry.get('OxyOSNotifications');
const DEFAULT_ACTION = 'expo.modules.notifications.actions.DEFAULT';
const notSupported = (what) => async () => {
  throw new Error(`expo-notifications' ${what} is not available on OxyOS`);
};
const emitter = () => new globalThis.expo.EventEmitter();
// What this process presented and the session still keeps: by identifier, with the session's
// number for it; and the categories' actions it declared.
const presented = new Map();
const numbered = new Map();
const categories = new Map();
let lastResponse = null;
// Posts waiting for the session's answer, by serial.
const posting = new Map();
let nextSerial = 0;

const ExpoNotificationsEmitter = emitter();
Object.assign(ExpoNotificationsEmitter, {
  getLastNotificationResponse: () => lastResponse,
  getLastNotificationResponseAsync: async () => lastResponse,
  clearLastNotificationResponse: () => {
    lastResponse = null;
  },
  clearLastNotificationResponseAsync: async () => {
    lastResponse = null;
  },
});

// The application's handler decides how a notification it presents is shown (expo-notifications
// subscribes a default one that shows it): asked as Expo's native side asks, answered with
// handleNotificationAsync, within three seconds.
const ExpoNotificationsHandlerModule = emitter();
const handling = new Map();
ExpoNotificationsHandlerModule.handleNotificationAsync = async (id, behavior) => {
  handling.get(id)?.(behavior);
};
const behaviorFor = (identifier, notification) =>
  new Promise((resolve) => {
    const timeout = setTimeout(() => {
      handling.delete(identifier);
      ExpoNotificationsHandlerModule.emit('onHandleNotificationTimeout', { id: identifier, notification });
      resolve(null);
    }, 3000);
    handling.set(identifier, (behavior) => {
      clearTimeout(timeout);
      handling.delete(identifier);
      resolve(behavior);
    });
    ExpoNotificationsHandlerModule.emit('onHandleNotification', { id: identifier, notification });
  });

const actionsOf = (content) => {
  const actions = categories.get(content.categoryIdentifier)?.actions ?? [];
  if (actions.length > 3) throw new Error('OxyOS shows at most 3 actions on a notification');
  return actions.map((action) => ({
    id: String(action.identifier),
    label: String(action.buttonTitle),
    // A text input makes it a reply; its placeholder is the field's.
    reply: action.textInput ? String(action.textInput.placeholder ?? '') : null,
    background: action.options?.opensAppToForeground === false,
  }));
};

const ExpoNotificationScheduler = emitter();
Object.assign(ExpoNotificationScheduler, {
  scheduleNotificationAsync: async (identifier, content, trigger) => {
    if (!notifications) throw new Error('OxyOS has no notifications here: the host is not connected');
    if (trigger !== null) {
      throw new Error('OxyOS presents notifications now (a null trigger); scheduled triggers are not available');
    }
    const data = JSON.stringify(content.data ?? {});
    if (data.length > 4096) throw new Error('A notification keeps at most 4096 bytes of data on OxyOS');
    const notification = {
      date: Date.now(),
      request: { identifier, content: { ...content, data: content.data ?? {} }, trigger: null },
    };
    // A handler that answers nothing in time shows it as expo-notifications' own default handler
    // does: a banner, in the list, with a sound.
    const behavior = (await behaviorFor(identifier, notification)) ?? {
      shouldShowBanner: true,
      shouldShowList: true,
      shouldPlaySound: true,
    };
    const banner = !!(behavior.shouldShowBanner || behavior.shouldShowAlert);
    // The system's notification sound (docs/adr/0044-system-sounds.mdx), as the handler's
    // shouldPlaySound asks; `sound: false` in the content silences it. A sound of the application's
    // own (a file name) plays the system's: OxyOS plays no application's files.
    const sound = content.sound !== false && !!behavior.shouldPlaySound;
    if (banner || behavior.shouldShowList) {
      const serial = ++nextSerial;
      const answer = new Promise((resolve) => posting.set(serial, resolve));
      const sent = notifications.post(serial, {
        identifier,
        title: content.title ?? '',
        subtitle: content.subtitle ?? '',
        body: content.body ?? '',
        data,
        actions: actionsOf(content),
        banner,
        sound,
      });
      if (!sent) {
        posting.delete(serial);
        throw new Error('OxyOS could not post the notification');
      }
      const { outcome, notification: number } = await answer;
      if (outcome !== 'posted') {
        throw new Error(
          outcome === 'throttled'
            ? 'OxyOS refused the notification: too many in a short time'
            : 'OxyOS refused the notification: the session did not start this application',
        );
      }
      // The same identifier again replaces it, as the session does.
      const replaced = presented.get(identifier);
      if (replaced) numbered.delete(replaced.number);
      presented.set(identifier, { number, notification });
      numbered.set(number, identifier);
    }
    ExpoNotificationsEmitter.emit('onDidReceiveNotification', notification);
    return identifier;
  },
  getAllScheduledNotificationsAsync: async () => [],
  cancelScheduledNotificationAsync: async () => {},
  cancelAllScheduledNotificationsAsync: async () => {},
});

const ExpoNotificationPresenter = emitter();
Object.assign(ExpoNotificationPresenter, {
  getPresentedNotificationsAsync: async () => [...presented.values()].map(({ notification }) => notification),
  dismissNotificationAsync: async (identifier) => {
    const shown = presented.get(identifier);
    if (shown) notifications?.withdraw(shown.number);
  },
  dismissAllNotificationsAsync: async () => notifications?.withdraw(0),
});

// The session decides how notifications show (Do Not Disturb, the lock screen); an application the
// session started may post, and there is nothing to ask.
// Permissions (docs/adr/0030-installing-applications.mdx, decision 8): a system or development
// application may post; an installed version only once its account granted `notifications`, asked
// in the system's sheet at its first requestPermissionsAsync, kept by the install service. Denied,
// expo-notifications' own answer says so, and the host refuses its posts.
const granted = { status: 'granted', granted: true, canAskAgain: true, expires: 'never' };
const denied = { status: 'denied', granted: false, canAskAgain: false, expires: 'never' };
const undetermined = { status: 'undetermined', granted: false, canAskAgain: true, expires: 'never' };
const notificationsAllowed = () => {
  const launched = launch();
  return !launched || !launched.narrowed || launched.capabilities.includes('notifications');
};
let notificationsAsked = false;
const ExpoNotificationPermissionsModule = emitter();
Object.assign(ExpoNotificationPermissionsModule, {
  getPermissionsAsync: async () =>
    notificationsAllowed() ? granted : notificationsAsked ? denied : undetermined,
  requestPermissionsAsync: async () => {
    if (notificationsAllowed()) return granted;
    notificationsAsked = true;
    return (await requestCapability('notifications')) ? granted : denied;
  },
});

const ExpoNotificationCategoriesModule = emitter();
Object.assign(ExpoNotificationCategoriesModule, {
  getNotificationCategoriesAsync: async () => [...categories.values()],
  setNotificationCategoryAsync: async (identifier, actions, options) => {
    const category = { identifier, actions, options };
    categories.set(identifier, category);
    return category;
  },
  deleteNotificationCategoryAsync: async (identifier) => categories.delete(identifier),
});

// The session's response to a press, each once (the host keeps them until taken): the
// notification as it was presented (or, for an application started by the press, as the session
// kept it), the action and the reply.
const respond = () => {
  if (!notifications) return;
  for (const taken of notifications.takeResponses()) {
    const identifier = taken.identifier;
    const shown = presented.get(identifier);
    presented.delete(identifier);
    numbered.delete(taken.notification);
    let data = {};
    try {
      data = taken.data ? JSON.parse(taken.data) : {};
    } catch {}
    const notification = shown?.notification ?? {
      date: Date.now(),
      request: {
        identifier,
        content: { title: taken.title || null, subtitle: taken.subtitle || null, body: taken.body || null, data },
        trigger: null,
      },
    };
    const response = {
      notification,
      actionIdentifier: taken.action === 0 ? DEFAULT_ACTION : taken.actionId,
      ...(taken.reply ? { userText: taken.reply } : {}),
    };
    lastResponse = response;
    ExpoNotificationsEmitter.emit('onDidReceiveNotificationResponse', response);
  }
};
DeviceEventEmitter.addListener('OxyOSNotificationResponse', respond);
DeviceEventEmitter.addListener('OxyOSNotificationResult', (result) => {
  posting.get(result.serial)?.(result);
  posting.delete(result.serial);
});
// Dismissed in the Notification Center, replaced, or withdrawn: no longer presented.
DeviceEventEmitter.addListener('OxyOSNotificationRemoved', ({ notification }) => {
  const identifier = numbered.get(notification);
  numbered.delete(notification);
  if (identifier !== undefined && presented.get(identifier)?.number === notification) {
    presented.delete(identifier);
    ExpoNotificationsEmitter.emit('onNotificationsDeleted', {});
  }
});
// A response that started this application came before JavaScript did.
respond();

const ExpoPushTokenManager = emitter();
Object.assign(ExpoPushTokenManager, {
  getDevicePushTokenAsync: notSupported('push notifications (getDevicePushTokenAsync)'),
  unregisterForNotificationsAsync: notSupported('push notifications (unregisterForNotificationsAsync)'),
});
const ExpoBadgeModule = emitter();
Object.assign(ExpoBadgeModule, {
  // The dock shows no badges yet: none is set.
  getBadgeCountAsync: async () => 0,
  setBadgeCountAsync: async () => false,
});
const ExpoBackgroundNotificationTasksModule = emitter();
Object.assign(ExpoBackgroundNotificationTasksModule, {
  registerTaskAsync: notSupported('background notification tasks'),
  unregisterTaskAsync: notSupported('background notification tasks'),
});
Object.assign(globalThis.expo.modules, {
  ExpoNotificationsEmitter,
  ExpoNotificationsHandlerModule,
  ExpoNotificationScheduler,
  ExpoNotificationPresenter,
  ExpoNotificationPermissionsModule,
  ExpoNotificationCategoriesModule,
  ExpoPushTokenManager,
  ExpoBadgeModule,
  ExpoBackgroundNotificationTasksModule,
  // Android's channels: none on OxyOS (expo-notifications asks only on Android).
  ExpoNotificationChannelManager: emitter(),
  ExpoNotificationChannelGroupManager: emitter(),
  // No push server registration: without `getRegistrationInfoAsync` nothing registers at import.
  NotificationsServerRegistrationModule: { setRegistrationInfoAsync: notSupported('push server registration') },
});
