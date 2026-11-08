// The host's module behind `notifications` (docs/adr/0026-notifications.mdx): the runtime's own,
// reached only by @oxyos/platform (docs/adr/0058-the-platform-api.mdx, decision 2). React Native's
// codegen makes the C++ base the host implements from this spec (decision 5), so a method the spec
// names and the host lacks, or a type that differs, does not compile.
//
// A post is answered by `OxyOSNotificationResult` ({serial, notification, outcome: 'posted' |
// 'throttled' | 'refused'}); `OxyOSNotificationRemoved` ({notification}) says the session no
// longer keeps one; `OxyOSNotificationResponse` says responses wait, which `takeResponses` takes,
// each once. Every call but `takeResponses` throws where the host has no connection to the
// compositor.
import type { TurboModule } from 'react-native';
import { TurboModuleRegistry } from 'react-native';

export type NotificationAction = {
  id: string;
  label: string;
  /** A reply's field placeholder: the action is a reply. Null: a button. */
  reply: string | null;
  /** Answered without bringing the application forward. */
  background: boolean;
};

export type PostedNotification = {
  identifier: string;
  title: string;
  subtitle: string;
  body: string;
  /** The application's data, as JSON (at most 4096 bytes). */
  data: string;
  actions: Array<NotificationAction>;
  banner: boolean;
  sound: boolean;
};

export type TakenResponse = {
  /** The session's number for the notification answered. */
  notification: number;
  /** 0: the notification itself; else the action's place, from 1. */
  action: number;
  identifier: string;
  title: string;
  subtitle: string;
  body: string;
  data: string;
  actionId: string;
  reply: string;
};

export interface Spec extends TurboModule {
  /** Sends `notification`, its answer to come naming `serial`: 'sent'; 'notPermitted' where this
      application's account has not granted it `notifications` (ADR 0030, decision 8), nothing
      sent; 'notSent' where it could not be sent. */
  post(serial: number, notification: PostedNotification): string;
  /** Withdraws the session's notification `notification` of this application's; 0: all of them. */
  withdraw(notification: number): void;
  takeResponses(): Array<TakenResponse>;
  addListener(eventName: string): void;
  removeListeners(count: number): void;
}

export default TurboModuleRegistry.get<Spec>('OxyOSNotifications');
