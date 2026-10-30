/**
 * OxyOS side of the Expo modules Bloom and Oxy apps reach (expo-font, expo-asset beneath it,
 * expo-clipboard, expo-notifications, and expo-blur's views). expo-modules-core's own global
 * polyfill provides the module classes; the native work is done by the host's TurboModules
 * (OxyOSFontRegistry, OxyOSClipboard, OxyOSNotifications) and host components (expo-blur.h). Anything OxyOS cannot do fails loudly.
 */
import { installExpoGlobalPolyfill } from 'expo-modules-core/src/polyfill/dangerous-internal';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

installExpoGlobalPolyfill();

// Expo's native side makes version 4 UUIDs (expo-notifications names a notification by one); the
// polyfill's asks the web's crypto, which Hermes has not. Identifiers, not secrets: unique is enough.
globalThis.expo.uuidv4 = () =>
  'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
  });

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

// Expo's native views the host draws (expo-blur.h), by module and view, with the props each takes
// as its Android view definition names them; React Native's view props come with every view.
const expoViews = {
  'ExpoBlur.ExpoBlurView': ['blurTargetId', 'intensity', 'tint', 'blurReductionFactor', 'borderRadii', 'blurMethod'],
  'ExpoBlur.ExpoBlurTargetView': [],
};
globalThis.expo.getViewConfig = (moduleName, viewName) => {
  const props = expoViews[`${moduleName}.${viewName}`];
  if (!props) throw new Error(`Expo's ${moduleName} ${viewName ?? 'view'} is not available on OxyOS`);
  return { validAttributes: Object.fromEntries(props.map((prop) => [prop, true])), directEventTypes: {} };
};

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
    // A handler that answers nothing in time shows it, as Expo does.
    const behavior = (await behaviorFor(identifier, notification)) ?? { shouldShowBanner: true, shouldShowList: true };
    const banner = !!(behavior.shouldShowBanner || behavior.shouldShowAlert);
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
const granted = { status: 'granted', granted: true, canAskAgain: true, expires: 'never' };
const ExpoNotificationPermissionsModule = emitter();
Object.assign(ExpoNotificationPermissionsModule, {
  getPermissionsAsync: async () => granted,
  requestPermissionsAsync: async () => granted,
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
