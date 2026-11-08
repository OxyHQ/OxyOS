/**
 * Settings › General › Storage (docs/adr/0062-storage.mdx): what the volume holding the person's
 * home occupies, by what occupies it, as a Mac's Storage page shows it. The figures are the
 * platform's: the session measures the volume, the system's image, the Trash and each application
 * (`{storage: {}}`), and the search index says what the home's files occupy by kind and which are
 * the largest (ADR 0051). Nothing here walks a disk or keeps a list of its own.
 */

/** An application, as the session measures it. */
export type StorageApp = {
  id: string;
  name: string;
  /** Where its entry comes from (ADR 0030, decision 7). */
  origin: "system" | "installed" | "development";
  /** Its package's bytes, when the account has one from the install service. */
  package: number | null;
  /** Its own data's bytes (`oxyos_apps::OwnData`): what it keeps, and its caches. */
  data: number;
  caches: number;
  /** When the account last opened it, or installed it if never since (ms since 1970). */
  lastUsed: number | null;
  /** The person may remove it, through the store's uninstall (never a system application). */
  removable: boolean;
};

/** What the session measured (bytes): the volume, the system's, the Trash's, each application's. */
export type StorageMeasured = {
  /** The volume's label, if it has one. */
  name: string | null;
  capacity: number;
  /** Free for the person (`f_bavail`). */
  available: number;
  /** Written (`f_blocks − f_bfree`). */
  used: number;
  /** Kept free for the administrator, so the system goes on working on a full disk. */
  reserved: number;
  /** The system's image on this volume. */
  system: number;
  trash: number;
  /** The largest first. */
  apps: StorageApp[];
};

/** What the home's files occupy by kind, as the search index keeps them (ADR 0051). */
export type IndexedStorage = {
  kinds: Record<string, { files: number; bytes: number }>;
  /** The index is still being built: its figures grow. */
  indexing: boolean;
  /** The index holds only part of the home (the system's limit of watches). */
  partial: boolean;
};

/** One of the bar's categories, in the order a Mac's bar shows them. */
export type StorageCategory = "apps" | "documents" | "system" | "photos" | "trash" | "other";
export const STORAGE_CATEGORIES: readonly StorageCategory[] = ["apps", "documents", "system", "photos", "trash", "other"];

/** The kinds of files that are Photos: what the index classes as images and movies. */
const PHOTO_KINDS = new Set(["image", "movie"]);

/** The bar: the volume's capacity, what is free, and each category's bytes. */
export type StorageBar = {
  capacity: number;
  available: number;
  segments: { key: StorageCategory; bytes: number }[];
};

/** An application's bytes: its package and its own data. */
export function appTotal(app: StorageApp): number {
  return (app.package ?? 0) + app.data + app.caches;
}

/**
 * The bar (decision 2): every category's bytes, which add up to what is used, the reserve and all,
 * so the bar and the free space are the whole volume.
 *
 * - **Apps**: each application's package and own data.
 * - **Documents**: the home's files the index holds that are not Photos.
 * - **Photos**: the home's images and movies the index holds.
 * - **Trash**: what the Trash holds.
 * - **System**: the system's image on this volume, and the administrator's reserve.
 * - **Other**: the rest of what is used: everything no category names (hidden files, folders kept
 *   out of the index, other accounts, the system's logs and caches).
 *
 * What is measured is taken in the order the session measures it most surely (the Trash, the
 * applications, the files, the system's own estimate), each up to what is left of what is used: on
 * a volume whose files share blocks (hard links, copies made by reference), what is measured can
 * exceed what is used, and the bar never says more than the volume holds.
 */
export function storageBar(measured: StorageMeasured, indexed: IndexedStorage | null): StorageBar {
  let photos = 0;
  let documents = 0;
  for (const [kind, { bytes }] of Object.entries(indexed?.kinds ?? {})) {
    if (PHOTO_KINDS.has(kind)) photos += bytes;
    else documents += bytes;
  }
  const apps = measured.apps.reduce((sum, app) => sum + appTotal(app), 0);
  let left = Math.max(0, measured.used);
  const take = (bytes: number) => {
    const taken = Math.min(Math.max(0, bytes), left);
    left -= taken;
    return taken;
  };
  const taken = {
    trash: take(measured.trash),
    apps: take(apps),
    photos: take(photos),
    documents: take(documents),
    system: take(measured.system),
  };
  const bytes: Record<StorageCategory, number> = {
    ...taken,
    system: taken.system + Math.max(0, measured.reserved),
    other: left,
  };
  return {
    capacity: measured.capacity,
    available: measured.available,
    segments: STORAGE_CATEGORIES.map((key) => ({ key, bytes: bytes[key] })),
  };
}

/** How long an application goes unopened before Storage recommends removing it: 90 days. */
export const UNUSED_AFTER_DAYS = 90;
const DAY_MS = 24 * 60 * 60 * 1000;

/** The applications the person may remove that they have not opened in [`UNUSED_AFTER_DAYS`], the
    largest first. One the platform knows nothing of when it was used is never one. */
export function unusedApps(apps: readonly StorageApp[], now: number): StorageApp[] {
  return apps
    .filter((app) => app.removable && app.lastUsed !== null && now - app.lastUsed >= UNUSED_AFTER_DAYS * DAY_MS)
    .sort((a, b) => appTotal(b) - appTotal(a));
}

/** A file Review Files lists, as the index gives it. */
export type LargeFile = {
  uri: string;
  name: string;
  /** Its folder, from the home ("Documents › Work"). */
  location: string;
  size: number;
  /** ms since 1970. */
  modified: number | null;
  category: string;
};

/** The index's items as Review Files lists them: files with a place and a size, the largest
    first, each once. */
export function largeFiles(items: readonly Record<string, unknown>[]): LargeFile[] {
  const seen = new Set<string>();
  const files: LargeFile[] = [];
  for (const item of items) {
    const uri = typeof item.uri === "string" ? item.uri : null;
    const size = typeof item.size === "number" ? item.size : null;
    if (!uri || size === null || seen.has(uri) || uri.endsWith("/")) continue;
    seen.add(uri);
    files.push({
      uri,
      name: typeof item.title === "string" ? item.title : uri,
      location: typeof item.location === "string" ? item.location : "",
      size,
      modified: typeof item.modified === "number" ? item.modified : null,
      category: typeof item.category === "string" ? item.category : "other",
    });
  }
  return files.sort((a, b) => b.size - a.size);
}

/** How a removal through the store's uninstall ended: removed, or the person cancelled the
    sheet, or it failed (said). */
export type RemoveOutcome = "removed" | "cancelled" | "failed";

/** The session's answer to an uninstall, read: `{uninstalled}`, `{cancelled}`, anything else a
    failure. */
export function removeOutcomeOf(answer: unknown): RemoveOutcome {
  const keys = answer && typeof answer === "object" ? Object.keys(answer) : [];
  if (keys.includes("uninstalled")) return "removed";
  if (keys.includes("cancelled")) return "cancelled";
  return "failed";
}

/** The session's answer to `{storage: {}}`, read strictly; null when it is not one. */
export function measuredOf(answer: unknown): StorageMeasured | null {
  const told = (answer as { storage?: Record<string, unknown> } | null)?.storage;
  if (!told || typeof told !== "object") return null;
  const number = (value: unknown) => (typeof value === "number" && Number.isFinite(value) ? value : 0);
  const apps = Array.isArray(told.apps) ? told.apps : [];
  return {
    name: typeof told.name === "string" && told.name ? told.name : null,
    capacity: number(told.capacity),
    available: number(told.available),
    used: number(told.used),
    reserved: number(told.reserved),
    system: number(told.system),
    trash: number(told.trash),
    apps: apps
      .filter((app): app is Record<string, unknown> => !!app && typeof app === "object" && typeof app.id === "string")
      .map((app) => ({
        id: app.id as string,
        name: typeof app.name === "string" ? app.name : (app.id as string),
        origin: app.origin === "system" || app.origin === "installed" ? app.origin : "development",
        package: typeof app.package === "number" ? app.package : null,
        data: number(app.data),
        caches: number(app.caches),
        lastUsed: typeof app.lastUsed === "number" ? app.lastUsed : null,
        removable: app.removable === true,
      })),
  };
}

/** The search index's answer to `{storage: {}}`, read; null when it is not one. */
export function indexedOf(answer: unknown): IndexedStorage | null {
  const told = answer as { kinds?: unknown; indexing?: unknown; partial?: unknown } | null;
  if (!told || typeof told.kinds !== "object" || told.kinds === null) return null;
  const kinds: IndexedStorage["kinds"] = {};
  for (const [kind, value] of Object.entries(told.kinds as Record<string, unknown>)) {
    const { files, bytes } = (value ?? {}) as { files?: unknown; bytes?: unknown };
    if (typeof bytes === "number") kinds[kind] = { files: typeof files === "number" ? files : 0, bytes };
  }
  return { kinds, indexing: told.indexing === true, partial: told.partial === true };
}

/**
 * What Settings › Storage asks of the platform (docs/adr/0062-storage.mdx). Each is asked when the
 * page is shown, or after one of its own changes; nothing is followed.
 */
export type StorageActions = {
  /** The session's measurement: the volume, the system, the Trash and each application. */
  measure(): Promise<StorageMeasured>;
  /** What the home's files occupy by kind, from the search index; null without one. */
  indexed(): Promise<IndexedStorage | null>;
  /** Told whenever the index or its state changed (built further, done): while it is still being
      built, the page asks again, so Documents and Photos grow as it goes. Returns the unsubscribe. */
  watchIndex?(listener: () => void): () => void;
  /** The home's largest files, the largest first, from the search index. */
  largest(limit: number): Promise<LargeFile[]>;
  /** Shows the file selected in its folder in the file manager, citing the press that asked. */
  reveal(uri: string): Promise<void>;
  /** Moves the file to the Trash, through the file service. */
  trash(uri: string): Promise<void>;
  /** Removes an application and its data through the store's uninstall, which the person
      confirms in the system's sheet. */
  remove(id: string): Promise<RemoveOutcome>;
};
