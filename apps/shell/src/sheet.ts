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

import type { WifiSecurity } from "./network";

export type Language = "es" | "en";

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

const words = {
  es: {
    install: (name: string) => `¿Instalar ${name}?`,
    update: (name: string) => `¿Actualizar ${name}?`,
    uninstall: (name: string) => `¿Eliminar ${name} y sus datos?`,
    consent: (name: string) => `${name} pide más acceso`,
    permission: (name: string, what: string) => `¿Permitir que ${name} ${what}?`,
    installButton: "Instalar",
    updateButton: "Actualizar",
    uninstallButton: "Eliminar",
    allow: "Permitir",
    dontAllow: "No permitir",
    cancel: "Cancelar",
    version: (version: string, size: string) => `Versión ${version} · ${size}`,
    uninstallBody: "Se eliminarán la app y sus datos en esta cuenta. Puedes volver a instalarla desde Atlas.",
    consentBody: (version: string) => `La versión ${version} usa:`,
    keyRotation: "El desarrollador firma esta versión con una clave nueva de su linaje.",
    later: "Se preguntará cuando la app lo use",
    unavailable: {
      install: "No se puede instalar la app",
      uninstall: "No se puede eliminar la app",
      update: "No se puede actualizar",
    },
    unavailableBody:
      "El servicio que instala las apps en este ordenador no está funcionando. Inténtalo de nuevo cuando vuelva a estar disponible.",
    ok: "Aceptar",
    emptyTrash: "¿Seguro que quieres eliminar permanentemente los ítems de la Papelera?",
    emptyTrashBody: (items: number) =>
      `${items === 1 ? "Se eliminará 1 ítem" : `Se eliminarán ${items} ítems`}. No puedes deshacer esta acción.`,
    emptyTrashButton: "Vaciar",
    wifiPassword: (ssid: string, kind: string) => `La red Wi-Fi “${ssid}” requiere una contraseña ${kind}.`,
    wifiIncorrect: "La contraseña no es correcta. Inténtalo de nuevo.",
    wifiJoin: "Conectarse",
    wifiField: "Contraseña",
    showPassword: "Mostrar contraseña",
    hidePassword: "Ocultar contraseña",
    wifiFailed: (ssid: string) => `No se ha podido conectar a la red “${ssid}”.`,
    wifiUnsupported: (ssid: string) => `OxyOS todavía no puede conectarse a “${ssid}”.`,
    wifiEnterprise: "Las redes empresariales (802.1X) aún no son compatibles.",
    wifiWep: "Las redes WEP no son compatibles: no son seguras.",
    wifiOtherUnsupported: "Esta red aún no es compatible.",
  },
  en: {
    install: (name: string) => `Install ${name}?`,
    update: (name: string) => `Update ${name}?`,
    uninstall: (name: string) => `Delete ${name} and its data?`,
    consent: (name: string) => `${name} asks for more access`,
    permission: (name: string, what: string) => `Allow ${name} to ${what}?`,
    installButton: "Install",
    updateButton: "Update",
    uninstallButton: "Delete",
    allow: "Allow",
    dontAllow: "Don't Allow",
    cancel: "Cancel",
    version: (version: string, size: string) => `Version ${version} · ${size}`,
    uninstallBody: "The app and its data in this account are deleted. You can install it again from Atlas.",
    consentBody: (version: string) => `Version ${version} uses:`,
    keyRotation: "The developer signs this version with a new key of its lineage.",
    later: "Asked when the app uses it",
    unavailable: {
      install: "Can't Install the App",
      uninstall: "Can't Delete the App",
      update: "Can't Update",
    },
    unavailableBody:
      "The service that installs apps on this computer isn't running. Try again once it's available.",
    ok: "OK",
    emptyTrash: "Are you sure you want to permanently erase the items in the Trash?",
    emptyTrashBody: (items: number) =>
      `${items === 1 ? "1 item is erased" : `${items} items are erased`}. You can't undo this action.`,
    emptyTrashButton: "Empty",
    wifiPassword: (ssid: string, kind: string) => `The Wi-Fi network “${ssid}” requires a ${kind} password.`,
    wifiIncorrect: "The password is incorrect. Try again.",
    wifiJoin: "Join",
    wifiField: "Password",
    showPassword: "Show password",
    hidePassword: "Hide password",
    wifiFailed: (ssid: string) => `Couldn't join the network “${ssid}”.`,
    wifiUnsupported: (ssid: string) => `OxyOS can't join “${ssid}” yet.`,
    wifiEnterprise: "Enterprise (802.1X) networks aren't supported yet.",
    wifiWep: "WEP networks aren't supported: they aren't secure.",
    wifiOtherUnsupported: "This network isn't supported yet.",
  },
};

/** Each capability's name, and what an application asking for it would do. */
const capabilities: Record<string, Record<Language, [string, string]>> = {
  network: { es: ["Internet", "conectarse a Internet"], en: ["The Internet", "connect to the Internet"] },
  notifications: { es: ["Notificaciones", "enviarte notificaciones"], en: ["Notifications", "send you notifications"] },
  files: { es: ["Tus archivos", "acceder a tus archivos"], en: ["Your files", "access your files"] },
  camera: { es: ["Cámara", "usar la cámara"], en: ["Camera", "use the camera"] },
  microphone: { es: ["Micrófono", "usar el micrófono"], en: ["Microphone", "use the microphone"] },
  location: { es: ["Ubicación", "usar tu ubicación"], en: ["Location", "use your location"] },
  "oxy-account": { es: ["Tu cuenta Oxy", "usar tu cuenta Oxy"], en: ["Your Oxy account", "use your Oxy account"] },
  "open-files": { es: ["Abrir archivos", "abrir archivos"], en: ["Opening files", "open files"] },
  "url-schemes": { es: ["Abrir enlaces", "abrir enlaces"], en: ["Opening links", "open links"] },
  background: { es: ["En segundo plano", "funcionar en segundo plano"], en: ["In the background", "run in the background"] },
  "login-item": { es: ["Al iniciar sesión", "abrirse al iniciar sesión"], en: ["At sign-in", "open at sign-in"] },
};

export function capabilityLabel(name: string, language: Language): string {
  return capabilities[name]?.[language][0] ?? name;
}

/** A size in bytes as a person reads it. */
export function size(bytes: number, language: Language): string {
  const units = language === "es" ? ["bytes", "kB", "MB", "GB"] : ["bytes", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const shown = unit === 0 ? String(value) : value.toFixed(value < 10 ? 1 : 0);
  return `${language === "es" ? shown.replace(".", ",") : shown} ${units[unit]}`;
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

/** What the sheet shows, in `language`; `names` the registry's names, for a permission's app. */
export function sheetView(sheet: SheetText, language: Language, names: Record<string, string> = {}): SheetView {
  const w = words[language];
  switch (sheet.kind) {
    case "install":
    case "update": {
      const plan = sheet.plan;
      const name = localized(plan.names, language, plan.name);
      return {
        title: sheet.kind === "update" ? w.update(name) : w.install(name),
        body: w.version(plan.version, size(plan.download, language)),
        lines: plan.capabilities.map((c) => ({
          label: capabilityLabel(c.name, language),
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
      const lines = (update.consent?.capabilities ?? []).map((c) => ({ label: capabilityLabel(c, language) }));
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
      const what = capabilities[sheet.capability]?.[language][1] ?? sheet.capability;
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
