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
 *
 * Pairing a Bluetooth device (docs/adr/0055-bluetooth.mdx), the session, the machine's pairing
 * agent, asks the person to compare a code (`bluetooth-confirm`), to let a device pair
 * (`bluetooth-authorize`) or use a service (`bluetooth-service`), shows a code to type on the
 * device (`bluetooth-display`, which only Cancel answers: the session takes it away once paired),
 * asks for the code the device shows or its PIN (`bluetooth-code`) and says a connection failed
 * (`bluetooth-failed`). The code typed goes to the session alone, as a password does.
 *
 * An application creating or using a passkey (docs/adr/0050-keychain-autofill-and-passkeys.mdx,
 * decision 7), the session asks the person to verify (`passkey`): "Use your passkey for
 * github.com?" or "Create a passkey for …", the application asking (by its id, named as the
 * registry names it), the account (a chooser when several passkeys match) and the Oxy account's
 * password, typed in the sheet's own secure field, which goes to the session alone and on to the
 * accounts' service, never to the application. A wrong one asks again saying so; too many say how
 * long to wait. A relying party where the person already has one (`excluded`) or has none
 * (`none`) is only said, with one button.
 *
 * An application that quit unexpectedly again within ten minutes
 * (docs/adr/0061-crashes-and-resetting-an-application.mdx), the session says so (`crash`), its
 * keyboard on Reopen: Reopen (the answer), Ignore, Reset… (which asks the session to reset it, and
 * the session asks the person in a sheet of its own, `reset`) and Report…, which shows the report
 * the session wrote, kept on this computer, to copy. Nothing is ever sent anywhere.
 */

import type { Language, ShellMessages } from "./messages";
import type { WifiSecurity } from "./network";
import type { BluetoothKind } from "./bluetooth";

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
  | { kind: "wifi-unsupported"; ssid: string; security: WifiSecurity }
  // Bluetooth (docs/adr/0055-bluetooth.mdx): `name` and `device` (its kind) say which device.
  | { kind: "bluetooth-confirm"; name: string; device: BluetoothKind; code: string }
  | { kind: "bluetooth-authorize"; name: string; device: BluetoothKind }
  | { kind: "bluetooth-service"; name: string; device: BluetoothKind; service: BluetoothService }
  | { kind: "bluetooth-display"; name: string; device: BluetoothKind; code: string }
  | { kind: "bluetooth-code"; name: string; device: BluetoothKind; code: BluetoothCodeKind }
  | { kind: "bluetooth-failed"; name: string; device: BluetoothKind; reason: string }
  // An application that quit unexpectedly (docs/adr/0061-crashes-and-resetting-an-application.mdx):
  // `times` in the last ten minutes, its report and where the session keeps it, shown at the notice
  // or at the report (the notification's Report…).
  | {
      kind: "crash";
      id: string;
      name: string;
      times: number;
      report: string;
      file?: string;
      show: "notice" | "report";
    }
  // Resetting an application: what is erased, and whether it quits first.
  | { kind: "reset"; id: string; name: string; running: boolean }
  // Passkeys (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7): `id` the asking
  // application's desktop entry id, `rpId` the relying party (a domain name).
  | PasskeySheet;

/** What a passkey's sheet asks: to make one, to use one, or only says one is there already or none
    is. */
export type PasskeyMode = "create" | "get" | "excluded" | "none";

export type PasskeySheet = {
  kind: "passkey";
  mode: PasskeyMode;
  id: string;
  rpId: string;
  userName?: string;
  choices: { userName: string; displayName: string }[];
  /** The password given was not right. */
  incorrect: boolean;
  /** Too many were not: seconds to wait. */
  retryAfter?: number;
  /** Asked again, the account the person had chosen. */
  chosen?: number;
};

/** What a paired device that is not trusted asks to use. */
export type BluetoothService = "audio" | "input" | "phone" | "files" | "network" | "other";
/** The code a pairing asks for: a legacy PIN (1 to 16 characters), or the passkey the other device
    shows (digits, at most 6). */
export type BluetoothCodeKind = "pin" | "passkey";

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
  /** Empty for a sheet only Cancel answers (a code shown to type on a device): its one button,
      `refuse`, in the answer's place. */
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
  /** A code shown large, to compare or to type on a device. */
  code?: string;
  /** A field for a pairing code, whose text is the answer (`accept` enabled once it is a valid
      one, Return submitting it): its label, and what it takes. */
  codeField?: { label: string; kind: BluetoothCodeKind };
  /** A passkey's chooser (docs/adr/0050-keychain-autofill-and-passkeys.mdx): the accounts the
      person may use, the first chosen until they choose another. Its rows' label is the list's. */
  choices?: { label: string; rows: { title: string; detail: string }[] };
  /** Which password the field takes: a Wi-Fi network's (8 to 63 bytes, or 64 hexadecimal digits),
      or the Oxy account's (anything not empty). */
  passwordKind?: "wifi" | "account";
  /** What else the sheet offers, beside its answer (an application that quit: Reset…, Report…):
      buttons of their own at its foot's start. */
  actions?: { id: SheetAction; label: string }[];
  /** A report to show (an application that quit): its title, its text, its buttons' names, and
      whether the sheet opens at it. */
  report?: { title: string; text: string; copy: string; copied: string; done: string; first: boolean };
  /** Whether the keyboard starts on its answer, a Mac's default button (Space presses it). */
  focusAnswer?: boolean;
};

/** Whether what the password field holds can be submitted, by the password it takes. */
export function submittablePassword(password: string, kind: "wifi" | "account" = "wifi"): boolean {
  return kind === "account" ? password.length > 0 : validWifiPassword(password);
}

/** What a sheet offers beside its answer. */
export type SheetAction = "reset" | "report";

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

/** Whether a pairing code can be one: a passkey is digits, at most six; a PIN 1 to 16 characters,
    counted in UTF-8 bytes as BlueZ takes it. The sheet's Pair button waits for one. */
export function validBluetoothCode(code: string, kind: BluetoothCodeKind): boolean {
  if (kind === "passkey") return /^[0-9]{1,6}$/.test(code);
  const length = utf8Length(code);
  return length >= 1 && length <= 16;
}

/** What a pairing code's field keeps of what was typed: a passkey's digits, at most six; a PIN, at
    most 16 bytes. */
export function bluetoothCodeTyped(text: string, kind: BluetoothCodeKind): string {
  if (kind === "passkey") return text.replace(/[^0-9]/g, "").slice(0, 6);
  let kept = "";
  for (const char of text) {
    if (utf8Length(kept + char) > 16) break;
    kept += char;
  }
  return kept;
}

/** A code as a screen reader says it, digit by digit: "1 2 3 4 5 6". */
export function spokenCode(code: string): string {
  return [...code].join(" ");
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
    case "bluetooth-confirm":
      return {
        title: w.bluetoothConfirm(sheet.name),
        body: "",
        lines: [],
        accept: w.bluetoothPair,
        refuse: w.cancel,
        destructive: false,
        code: sheet.code,
      };
    case "bluetooth-authorize":
      return {
        title: w.bluetoothAuthorize(sheet.name),
        body: "",
        lines: [],
        accept: w.bluetoothPair,
        refuse: w.cancel,
        destructive: false,
      };
    case "bluetooth-service":
      return {
        title: w.bluetoothService(sheet.name, w.bluetoothServices[sheet.service] ?? w.bluetoothServices.other),
        body: "",
        lines: [],
        accept: w.allow,
        refuse: w.dontAllow,
        destructive: false,
      };
    case "bluetooth-display":
      return {
        title: w.bluetoothDisplay(sheet.name),
        body: "",
        lines: [],
        accept: "",
        refuse: w.cancel,
        destructive: false,
        code: sheet.code,
      };
    case "bluetooth-code": {
      const pin = sheet.code === "pin";
      return {
        title: pin ? w.bluetoothPin(sheet.name) : w.bluetoothPasskey(sheet.name),
        body: pin ? w.bluetoothPinHint : "",
        lines: [],
        accept: w.bluetoothPair,
        refuse: w.cancel,
        destructive: false,
        codeField: { label: pin ? w.bluetoothPinField : w.bluetoothCodeField, kind: pin ? "pin" : "passkey" },
      };
    }
    case "passkey":
      return passkeyView(sheet, messages, names);
    case "crash": {
      const again = sheet.times > 1;
      return {
        title: again ? w.crashAgain(sheet.name) : w.crash(sheet.name),
        body: again ? w.crashAgainBody(sheet.times) : w.crashBody,
        lines: [],
        accept: w.reopen,
        refuse: w.ignore,
        destructive: false,
        actions: [
          { id: "reset", label: w.resetButtonEllipsis },
          { id: "report", label: w.reportButton },
        ],
        report: {
          title: w.reportTitle(sheet.name),
          text: sheet.report,
          copy: w.copy,
          copied: w.copied,
          done: w.done,
          first: sheet.show === "report",
        },
        focusAnswer: true,
      };
    }
    case "reset":
      return {
        title: w.reset(sheet.name),
        body: sheet.running ? `${w.resetBody(sheet.name)} ${w.resetQuits(sheet.name)}` : w.resetBody(sheet.name),
        lines: [],
        accept: w.resetButton,
        refuse: w.cancel,
        destructive: true,
      };
    case "bluetooth-failed":
      return {
        title: w.bluetoothFailed(sheet.name),
        body:
          sheet.reason === "rejected"
            ? w.bluetoothRejected
            : sheet.reason === "timeout"
              ? w.bluetoothTimeout
              : w.bluetoothOtherFailure,
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

/** A passkey's sheet: what it asks, for which relying party, by which application. */
function passkeyView(sheet: PasskeySheet, messages: ShellMessages, names: Record<string, string>): SheetView {
  const w = messages.sheet;
  const app = names[sheet.id] ?? sheet.id;
  const requested = w.passkeyRequestedBy(app);
  const tells = sheet.mode === "excluded" || sheet.mode === "none";
  if (tells) {
    return {
      title: sheet.mode === "excluded" ? w.passkeyExcluded(sheet.rpId) : w.passkeyNone(sheet.rpId),
      body: `${sheet.mode === "excluded" ? w.passkeyExcludedBody : w.passkeyNoneBody} ${requested}`,
      lines: [],
      accept: w.ok,
      refuse: "",
      destructive: false,
    };
  }
  const create = sheet.mode === "create";
  const several = !create && sheet.choices.length > 1;
  const account = create ? sheet.userName : (sheet.choices[0]?.userName ?? sheet.userName);
  const error =
    sheet.retryAfter !== undefined
      ? w.passkeyThrottled(sheet.retryAfter)
      : sheet.incorrect
        ? w.passkeyIncorrect
        : undefined;
  return {
    title: create ? w.passkeyCreate(sheet.rpId) : w.passkeyGet(sheet.rpId),
    body: [several ? "" : account ? w.passkeyAccount(account) : "", create ? w.passkeyCreateBody : "", requested]
      .filter(Boolean)
      .join(" "),
    lines: [],
    accept: w.passkeyContinue,
    refuse: w.cancel,
    destructive: false,
    password: { label: w.passkeyField, show: w.showPassword, hide: w.hidePassword },
    passwordKind: "account",
    error,
    choices: several
      ? {
          label: w.passkeyChoose,
          rows: sheet.choices.map((choice) => ({ title: choice.userName, detail: choice.displayName })),
        }
      : undefined,
  };
}
