// `notifications` (docs/adr/0058-the-platform-api.mdx, decision 1; docs/adr/0026-notifications.mdx):
// an application's notifications are the session's. The session keeps them (the Notification
// Center), shows them as banners as the person's preferences allow (Do Not Disturb), and gives the
// application the response once when the person presses one; the application is its launch record,
// never a claim. Posting needs the `notifications` capability (ADR 0030, decision 8), which the host
// checks before it sends anything.
import type { Spec, TakenResponse } from '../specs/NativeOxyOSNotifications.ts';
import { PlatformError } from './errors.ts';
import type { CallOptions } from './frameworks.ts';
import { hostCall } from './host.ts';
import type { Events } from './session.ts';

/** The host's module (OxyOSNotifications), as its spec says it. */
export type NotificationsModule = Pick<Spec, 'post' | 'withdraw' | 'takeResponses'>;

/** A button on a notification, or a reply's field. */
export interface NotificationAction {
  /** The application's name for it, given back in a response. */
  id: string;
  label: string;
  /** A reply: the field's placeholder. None: a button. */
  reply?: string;
  /** Answered without bringing the application forward. Default false. */
  background?: boolean;
}

/** The most actions a notification shows, and the most data it keeps (as JSON, in bytes). */
export const MAX_NOTIFICATION_ACTIONS = 3;
export const MAX_NOTIFICATION_DATA = 4096;

export interface Notification {
  /** The application's name for it: posting the same identifier again replaces it. */
  identifier: string;
  title?: string;
  subtitle?: string;
  body?: string;
  /** What the application keeps with it, given back in a response. */
  data?: Readonly<Record<string, unknown>>;
  actions?: readonly NotificationAction[];
  /** Shown as a banner, as well as kept in the Notification Center. Default true. */
  banner?: boolean;
  /** With the system's notification sound (docs/adr/0044-system-sounds.mdx). Default true. */
  sound?: boolean;
}

/** The person's answer to a notification, given once. */
export interface NotificationResponse {
  /** The session's number for the notification. */
  notification: number;
  identifier: string;
  title: string;
  subtitle: string;
  body: string;
  data: Record<string, unknown>;
  /** The action pressed, by its id; null for the notification itself. */
  action: string | null;
  /** What the person typed in a reply's field; null for none. */
  reply: string | null;
}

/** What the host tells of a post (`OxyOSNotificationResult`). */
interface PostResult {
  serial: number;
  notification: number;
  outcome: 'posted' | 'throttled' | 'refused';
}

function responseOf(taken: TakenResponse): NotificationResponse {
  let data: Record<string, unknown> = {};
  try {
    const parsed: unknown = taken.data ? JSON.parse(taken.data) : {};
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) data = parsed as Record<string, unknown>;
  } catch {
    // Data that is not JSON's is no data.
  }
  return {
    notification: taken.notification,
    identifier: taken.identifier,
    title: taken.title,
    subtitle: taken.subtitle,
    body: taken.body,
    data,
    action: taken.action === 0 ? null : taken.actionId,
    reply: taken.reply ? taken.reply : null,
  };
}

export function createNotifications(native: NotificationsModule | null | undefined, events: Events) {
  const unavailable = () => new PlatformError('notifications', 'unavailable');
  const posting = new Map<number, (result: PostResult) => void>();
  let nextSerial = 0;
  const responders = new Set<(response: NotificationResponse) => void>();

  // Responses wait in the host until taken, each once: taken only while something follows them, so
  // one that started the application waits for the application's first follower.
  const respond = () => {
    if (!native || responders.size === 0) return;
    for (const taken of native.takeResponses()) {
      const response = responseOf(taken);
      for (const responder of [...responders]) responder(response);
    }
  };
  if (native) {
    events.addListener('OxyOSNotificationResult', (result: PostResult) => {
      posting.get(result.serial)?.(result);
      posting.delete(result.serial);
    });
    events.addListener('OxyOSNotificationResponse', respond);
  }

  /** A host call of the module's (they throw at once where the host has no compositor). */
  const call = (start: () => void, options?: CallOptions): Promise<void> => {
    if (!native) return Promise.reject(unavailable());
    return hostCall(
      'notifications',
      () => {
        start();
        return Promise.resolve();
      },
      options,
    );
  };

  return {
    available: Boolean(native),
    /**
     * Posts `notification` now; resolves with the session's number for it. Rejects with
     * `notPermitted` where the application may not post (its account has not granted
     * `notifications`, or the session did not start it: `reason` `notLaunched`), `busy` (`reason`
     * `throttled`) for too many in a short time, `invalid` for more than three actions or data
     * beyond 4096 bytes, `unavailable` where the host has no compositor. Aborted, it rejects with
     * `cancelled` at once and the session's answer is dropped.
     */
    async post(notification: Notification, options?: CallOptions): Promise<number> {
      if (!native) throw unavailable();
      const actions = notification.actions ?? [];
      if (actions.length > MAX_NOTIFICATION_ACTIONS) throw new PlatformError('notifications', 'invalid', 'actions');
      const data = JSON.stringify(notification.data ?? {});
      if (data.length > MAX_NOTIFICATION_DATA) throw new PlatformError('notifications', 'invalid', 'data');
      const serial = ++nextSerial;
      const result = await hostCall(
        'notifications',
        () => {
          const sent = native.post(serial, {
            identifier: notification.identifier,
            title: notification.title ?? '',
            subtitle: notification.subtitle ?? '',
            body: notification.body ?? '',
            data,
            actions: actions.map(({ id, label, reply, background }) => ({
              id: String(id),
              label: String(label),
              reply: reply === undefined ? null : String(reply),
              background: background === true,
            })),
            banner: notification.banner !== false,
            sound: notification.sound !== false,
          });
          if (sent === 'notPermitted') return Promise.reject(new PlatformError('notifications', 'notPermitted'));
          if (sent !== 'sent') return Promise.reject(new PlatformError('notifications', 'unavailable', 'notSent'));
          return new Promise<PostResult>((resolve) => posting.set(serial, resolve));
        },
        options,
        () => posting.delete(serial),
      );
      if (result.outcome === 'posted') return result.notification;
      if (result.outcome === 'throttled') throw new PlatformError('notifications', 'busy', 'throttled');
      throw new PlatformError('notifications', 'notPermitted', 'notLaunched');
    },
    /** Withdraws the session's notification `notification` (posted by this application). */
    withdraw: (notification: number, options?: CallOptions) =>
      call(() => native?.withdraw(notification), options),
    /** Withdraws every notification this application posted that the session keeps. */
    withdrawAll: (options?: CallOptions) => call(() => native?.withdraw(0), options),
    /** The person's answers to this application's notifications, each told once; one that started
        the application is told to its first follower. The function returned stops following. */
    onResponse(listener: (response: NotificationResponse) => void): () => void {
      responders.add(listener);
      respond();
      return () => {
        responders.delete(listener);
      };
    },
    /** The session no longer keeps one of this application's notifications (dismissed in the
        Notification Center, replaced, or withdrawn): its number. */
    onRemoved(listener: (notification: number) => void): () => void {
      if (!native) return () => {};
      const subscription = events.addListener('OxyOSNotificationRemoved', ({ notification }: { notification: number }) =>
        listener(notification),
      );
      return () => subscription.remove();
    },
  };
}

export type Notifications = ReturnType<typeof createNotifications>;
