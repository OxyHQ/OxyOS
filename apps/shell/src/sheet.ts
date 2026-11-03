/**
 * The system's sheets (docs/adr/0030-installing-applications.mdx, decisions 8, 9 and 16): what the
 * session asks the person before it installs, removes, applies an update with new capabilities or
 * lets an application use a capability asked at first use. The session sends the shell alone each
 * sheet (`OxyOSSheet`); the shell draws it over everything but the lock screen, and only the
 * shell's own press answers it, so no application can confirm one for the person. No password is
 * asked: the sheet is the proof a person chose.
 *
 * When the person asked for a change and the install service is not running, the session sends a
 * sheet that says so (`unavailable`): one button, nothing to confirm.
 *
 * Joining a Wi-Fi network (docs/adr/0038-networks.mdx, decisions 4 and 5), the session asks for its
 * password (`wifi-password`, again with `incorrect` after a wrong one), says it could not join
 * (`wifi-failed`) or that OxyOS cannot join such a network (`wifi-unsupported`). The password is
 * typed into the sheet's own secure field and goes to the session alone; nothing here keeps it.
 */

import type { Language, ShellMessages } from "./messages";
import type { WifiSecurity } from "./network";

export type { Language } from "./messages";

/** A sheet as the session describes it (JSON). */
export type SheetText =
  | { kind: "install" | "update"; plan: Plan }
  | { kind: "uninstall"; id: string; name: string; version: string }
  | { kind: "consent"; update: AvailableUpdate }
  | { kind: "permission"; id: string; capability: string; name?: string }
  | { kind: "unavailable"; action: "install" | "uninstall" | "update" }
  // The dock's Trash emptied (docs/adr/0034-the-dock.mdx): how many items go for good.
  | { kind: "empty-trash"; items: number }
  // Wi-Fi (docs/adr/0038-networks.mdx): a network's password, a join that failed, a network
  // OxyOS cannot join.
  | { kind: "wifi-password"; ssid: string; security: WifiSecurity; incorrect: boolean }
  | { kind: "wifi-failed"; ssid: string }
  | { kind: "wifi-unsupported"; ssid: string; security: WifiSecurity };

export type Plan = {
  id: string;
  name: string;
  names: Record<string, string>;
  version: string;
  download: number;
  capabilities: { name: string; purpose: Record<string, string>; askedAtFirstUse: boolean }[];
  newCapabilities: string[];
  keyRotation: boolean;
  update: boolean;
};

export type AvailableUpdate = {
  id: string;
  name: string;
  names: Record<string, string>;
  toVersion: string;
  download: number;
  releaseNotes: Record<string, string>;
  consent: { capabilities: string[]; keyRotation: boolean } | null;
};

/** What the sheet shows. */
export type SheetView = {
  title: string;
  body: string;
  /** Each capability: its name and what the application says it is for. */
  lines: { label: string; detail?: string }[];
  accept: string;
  /** Empty for a sheet that only tells: it has its one button, `accept`. */
  refuse: string;
  /** A removal deletes the account's data: its button says so. */
  destructive: boolean;
  /** A secure field for a password, whose text is the answer (`accept` enabled once it is a valid
      Wi-Fi password, Return submitting it); its label, and its reveal button's names. */
  password?: { label: string; show: string; hide: string };
  /** What went wrong the last time, said prominently (a password that was not correct). */
  error?: string;
};

/** A capability's name, in the catalogue's language; its own name for one OxyOS does not know. */
export function capabilityLabel(name: string, messages: ShellMessages): string {
  return messages.sheet.capabilities[name]?.[0] ?? name;
}

/** A size in bytes as a person reads it, in the locale's digits ("1,5 MB", "1.5 MB"). */
export function size(bytes: number, messages: ShellMessages): string {
  const units = messages.sheet.units;
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const shown = messages.format.number(value, unit === 0 || value >= 10 ? 0 : 1);
  return `${shown} ${units[unit]}`;
}

/** A text in the person's language; else `fallback` (an app's own name), else English, else any. */
export function localized(texts: Record<string, string> | undefined, language: Language, fallback = ""): string {
  if (!texts) return fallback;
  return texts[language] ?? (fallback || texts.en || Object.values(texts)[0] || "");
}

/** A string's length in UTF-8 bytes, as the network daemon counts a passphrase. */
function utf8Length(text: string): number {
  let bytes = 0;
  for (const char of text) {
    const code = char.codePointAt(0)!;
    bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
  }
  return bytes;
}

/** Whether a WPA password can be one: a passphrase of 8 to 63 bytes, or the key itself, 64
    hexadecimal digits. The sheet's Join button waits for one. */
export function validWifiPassword(password: string): boolean {
  if (/^[0-9a-fA-F]{64}$/.test(password)) return true;
  const length = utf8Length(password);
  return length >= 8 && length <= 63;
}

/** The sheet the session sent, read; null for one the shell does not know (it is refused). */
export function readSheet(text: string): SheetText | null {
  try {
    const sheet = JSON.parse(text);
    if (sheet && typeof sheet === "object" && typeof sheet.kind === "string") return sheet as SheetText;
  } catch {}
  return null;
}

/** What the sheet shows, in the catalogue's language; `names` the registry's names, for a
    permission's app. */
export function sheetView(sheet: SheetText, messages: ShellMessages, names: Record<string, string> = {}): SheetView {
  const w = messages.sheet;
  const language = messages.language;
  switch (sheet.kind) {
    case "install":
    case "update": {
      const plan = sheet.plan;
      const name = localized(plan.names, language, plan.name);
      return {
        title: sheet.kind === "update" ? w.update(name) : w.install(name),
        body: w.version(plan.version, size(plan.download, messages)),
        lines: plan.capabilities.map((c) => ({
          label: capabilityLabel(c.name, messages),
          detail: c.askedAtFirstUse
            ? `${localized(c.purpose, language)}${localized(c.purpose, language) ? " · " : ""}${w.later}`
            : localized(c.purpose, language) || undefined,
        })),
        accept: sheet.kind === "update" ? w.updateButton : w.installButton,
        refuse: w.cancel,
        destructive: false,
      };
    }
    case "uninstall":
      return {
        title: w.uninstall(sheet.name),
        body: w.uninstallBody,
        lines: [],
        accept: w.uninstallButton,
        refuse: w.cancel,
        destructive: true,
      };
    case "consent": {
      const update = sheet.update;
      const name = localized(update.names, language, update.name);
      const lines = (update.consent?.capabilities ?? []).map((c) => ({ label: capabilityLabel(c, messages) }));
      if (update.consent?.keyRotation) lines.push({ label: w.keyRotation });
      return {
        title: w.consent(name),
        body: w.consentBody(update.toVersion),
        lines,
        accept: w.updateButton,
        refuse: w.cancel,
        destructive: false,
      };
    }
    case "empty-trash":
      return {
        title: w.emptyTrash,
        body: w.emptyTrashBody(sheet.items),
        lines: [],
        accept: w.emptyTrashButton,
        refuse: w.cancel,
        destructive: true,
      };
    case "permission": {
      const name = names[sheet.id] ?? sheet.name ?? sheet.id;
      const what = w.capabilities[sheet.capability]?.[1] ?? sheet.capability;
      return {
        title: w.permission(name, what),
        body: "",
        lines: [],
        accept: w.allow,
        refuse: w.dontAllow,
        destructive: false,
      };
    }
    case "unavailable":
      return {
        title: w.unavailable[sheet.action] ?? w.unavailable.install,
        body: w.unavailableBody,
        lines: [],
        accept: w.ok,
        refuse: "",
        destructive: false,
      };
    case "wifi-password":
      return {
        title: w.wifiPassword(sheet.ssid, sheet.security === "wpa3" ? "WPA3" : "WPA2"),
        body: "",
        lines: [],
        accept: w.wifiJoin,
        refuse: w.cancel,
        destructive: false,
        password: { label: w.wifiField, show: w.showPassword, hide: w.hidePassword },
        error: sheet.incorrect ? w.wifiIncorrect : undefined,
      };
    case "wifi-failed":
      return {
        title: w.wifiFailed(sheet.ssid),
        body: "",
        lines: [],
        accept: w.ok,
        refuse: "",
        destructive: false,
      };
    case "wifi-unsupported":
      return {
        title: w.wifiUnsupported(sheet.ssid),
        body:
          sheet.security === "wep"
            ? w.wifiWep
            : sheet.security === "enterprise"
              ? w.wifiEnterprise
              : w.wifiOtherUnsupported,
        lines: [],
        accept: w.ok,
        refuse: "",
        destructive: false,
      };
  }
}

/** Sheets waiting, the first shown: the session may ask several (an update's consents in turn). */
export type SheetQueue = { sheet: number; text: SheetText }[];

export function sheetArrived(queue: SheetQueue, sheet: number, text: string): SheetQueue {
  const read = readSheet(text);
  if (!read || queue.some((s) => s.sheet === sheet)) return queue;
  return [...queue, { sheet, text: read }];
}

export function sheetGone(queue: SheetQueue, sheet: number): SheetQueue {
  return queue.filter((s) => s.sheet !== sheet);
}
