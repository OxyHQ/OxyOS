/**
 * The system's sheets (docs/adr/0030-installing-applications.mdx, decisions 8, 9 and 16): what the
 * session asks the person before it installs, removes, applies an update with new capabilities or
 * lets an application use a capability asked at first use. The session sends the shell alone each
 * sheet (`OxyOSSheet`); the shell draws it over everything but the lock screen, and only the
 * shell's own press answers it, so no application can confirm one for the person. No password is
 * asked: the sheet is the proof a person chose.
 */

export type Language = "es" | "en";

/** A sheet as the session describes it (JSON). */
export type SheetText =
  | { kind: "install" | "update"; plan: Plan }
  | { kind: "uninstall"; id: string; name: string; version: string }
  | { kind: "consent"; update: AvailableUpdate }
  | { kind: "permission"; id: string; capability: string; name?: string };

type Plan = {
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

type AvailableUpdate = {
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
  refuse: string;
  /** A removal deletes the account's data: its button says so. */
  destructive: boolean;
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
