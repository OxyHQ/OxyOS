/**
 * Settings › Passwords (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 8), as a Mac's
 * Passwords: the Oxy Keychain of the person's account, which the accounts' service holds and
 * decrypts. Settings never holds the vault, a keyring or a recovery key beyond showing it once: it
 * asks the service item by item (`keychainList`, `keychainReveal` …), each answer read strictly,
 * and shows what it is told. The strong password is the service's generator's and the
 * verification code the service's own TOTP: no cryptography here.
 */
import type { PasswordsMessages } from "./messages";

/** Where the keychain stands for this account, as the service says (`keychainStatus`). */
export type KeychainState =
  | "locked"
  | "needsPreviousPassword"
  | "off"
  | "creating"
  | "joining"
  | "member"
  | "removed"
  | "unbound";

export type KeychainStatus = {
  state: KeychainState;
  /** Whether the account has a vault at Oxy; null when not known (offline, locked). */
  vault: boolean | null;
  /** Settings proved the person within the last five minutes of activity. */
  verified: boolean;
  /** The join's six-digit code, once both devices can show it. */
  code?: string;
  items?: number;
};

export type ItemKind = "login" | "passkey" | "totp" | "note" | "wifi";

/** An item as the list shows it: never a password, a code, a key or a note. */
export type ItemSummary = {
  itemId: string;
  type: ItemKind;
  title: string;
  username?: string;
  /** The hosts of a login's URLs, or a passkey's relying party. */
  domains: string[];
  /** A login with a verification code, or a code's own item. */
  hasCode: boolean;
  modifiedAt: string;
};

/** An item's fields, by type, as `keychainReveal` gives them and `keychainPut` takes them. */
export type ItemFields =
  | {
      type: "login";
      title: string;
      username: string;
      password: string;
      urls: string[];
      apps: string[];
      totp?: string | null;
      notes?: string | null;
    }
  | { type: "passkey"; rpId: string; rpName: string; userName: string; userDisplayName: string }
  | { type: "totp"; title: string; issuer?: string | null; account?: string | null; uri: string; notes?: string | null }
  | { type: "note"; title: string; body: string }
  | { type: "wifi"; ssid: string; password?: string | null; security: string; hidden: boolean };

export type ItemView = ItemFields & { itemId: string; createdAt: string; modifiedAt: string };

/** What can be written: everything but a passkey, which only the authenticator makes. */
export type ItemInput = Exclude<ItemFields, { type: "passkey" }> & { itemId?: string };

export type Enrollment = {
  enrollmentId: string;
  status: "waiting" | "code" | "approved";
  code?: string;
  name?: string;
  platform?: string;
  expiresAt?: string;
};

export type Member = { memberId: string; name: string; platform: string; self: boolean; orphaned: boolean };

export type Audit = { reusedCount: number; weakCount: number; reused: string[][]; weak: string[] };

export type Code = { code: string; period: number; remaining: number };

/** Why the service refused, as it names it. */
export type KeychainRefusal =
  | "notPermitted"
  | "invalidRequest"
  | "tooMany"
  | "failed"
  | "locked"
  | "offline"
  | "oxy"
  | "notMember"
  | "vaultExists"
  | "removed"
  | "needsPreviousPassword"
  | "verificationRequired"
  | "wrongPassword"
  | "throttled"
  | "recoveryKeyMismatch"
  | "notFound"
  | "denied"
  | "expired"
  | "conflict";

const REFUSALS: readonly KeychainRefusal[] = [
  "notPermitted",
  "invalidRequest",
  "tooMany",
  "failed",
  "locked",
  "offline",
  "oxy",
  "notMember",
  "vaultExists",
  "removed",
  "needsPreviousPassword",
  "verificationRequired",
  "wrongPassword",
  "throttled",
  "recoveryKeyMismatch",
  "notFound",
  "denied",
  "expired",
  "conflict",
];

/** What came of a request: its answer, the service's refusal, or the service not reached. */
export type Outcome<T> = { ok: T } | { refused: KeychainRefusal; retryAfter?: number } | { failed: string };

/** A request to the keychain, as the service's session socket takes it (Settings' messages). */
export type KeychainRequest =
  | { keychainStatus: Record<string, never> }
  | { keychainVerify: { password: string } }
  | { keychainCreate: Record<string, never> }
  | { keychainConfirmRecoveryKey: { recoveryKey: string } }
  | { keychainJoin: { method: "approval" } | { method: "recoveryKey"; recoveryKey: string } }
  | { keychainEnrollments: Record<string, never> }
  | { keychainApprove: { enrollmentId: string; confirm?: boolean } }
  | { keychainDeny: { enrollmentId: string } }
  | { keychainMembers: Record<string, never> }
  | { keychainRemoveMember: { memberId: string } }
  | { keychainList: Record<string, never> }
  | { keychainReveal: { itemId: string } }
  | { keychainPut: { item: ItemInput } }
  | { keychainDelete: { itemId: string } }
  | { keychainAudit: Record<string, never> }
  | { keychainRekey: { previous: string; current: string } }
  | { keychainLeave: Record<string, never> }
  | { keychainGenerate: Record<string, never> }
  | { keychainCode: { itemId: string } };

/** What Settings asks of the platform for the keychain. */
export type KeychainPlatform = {
  /** One request, its answer as the service told it, or {failed: {reason}} when it could not be
      asked. */
  ask: (request: KeychainRequest) => Promise<unknown>;
  /** Copies `text` marked sensitive (docs/adr/0007-clipboard.mdx, decision 6): clipboard managers
      keep none of it, and it is cleared after `SENSITIVE_CLEAR_SECONDS` if still there. */
  copySensitive: (text: string) => Promise<boolean>;
  /** Calls `listener` each time Settings' window becomes active or stops being (the session's lock
      leaves no window active, docs/adr/0016-windows.mdx), and at once with `false` if it is not
      active as it is called: Passwords asks whether it may stay open. Returns the unsubscribe;
      none where the platform cannot tell. */
  watchActive?: (listener: (active: boolean) => void) => () => void;
};

/** How long a copied password or code stays on the clipboard (ADR 0050 decision 8). */
export const SENSITIVE_CLEAR_SECONDS = 90;
/** How long Passwords stays open with nothing done in it (ADR 0050 decision 8). */
export const ACCESS_IDLE_MS = 5 * 60 * 1000;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const isString = (value: unknown): value is string => typeof value === "string";
const optionalString = (value: unknown): value is string | null | undefined =>
  value === undefined || value === null || typeof value === "string";
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(isString);

/** The one key of an answer such as {"keychainItems": …}, and what it holds. */
function single(told: unknown): [string, unknown] | null {
  if (typeof told === "string") return [told, null];
  if (!isRecord(told)) return null;
  const keys = Object.keys(told);
  return keys.length === 1 ? [keys[0]!, told[keys[0]!]] : null;
}

/** A refusal or a failure as an outcome, or null when the answer is neither. */
function notAnswered(told: unknown): Exclude<Outcome<never>, { ok: unknown }> | null {
  const one = single(told);
  if (!one) return { failed: "no answer" };
  const [key, value] = one;
  if (key === "refused" && isRecord(value)) {
    const reason = REFUSALS.find((known) => known === value.reason) ?? "failed";
    const retryAfter = value.retryAfter;
    return reason === "throttled" && typeof retryAfter === "number" && Number.isFinite(retryAfter) && retryAfter > 0
      ? { refused: reason, retryAfter: Math.ceil(retryAfter) }
      : { refused: reason };
  }
  if (key === "failed") {
    return { failed: isRecord(value) && isString(value.reason) ? value.reason : "failed" };
  }
  return null;
}

/** An answer read strictly: `key`'s value through `read`, else a failure (never an answer
    invented from a malformed one). */
function answered<T>(told: unknown, key: string, read: (value: unknown) => T | null): Outcome<T> {
  const refusal = notAnswered(told);
  if (refusal) return refusal;
  const [found, value] = single(told)!;
  if (found !== key) return { failed: `not ${key}` };
  const kept = read(value);
  return kept === null ? { failed: `not ${key}` } : { ok: kept };
}

const STATES: readonly KeychainState[] = [
  "locked",
  "needsPreviousPassword",
  "off",
  "creating",
  "joining",
  "member",
  "removed",
  "unbound",
];

export function statusFrom(value: unknown): KeychainStatus | null {
  if (!isRecord(value)) return null;
  const state = STATES.find((known) => known === value.state);
  if (!state || typeof value.verified !== "boolean") return null;
  if (value.vault !== null && typeof value.vault !== "boolean") return null;
  if (value.code !== undefined && !(isString(value.code) && /^\d{6}$/.test(value.code))) return null;
  if (value.items !== undefined && !(Number.isInteger(value.items) && (value.items as number) >= 0)) return null;
  return {
    state,
    vault: value.vault,
    verified: value.verified,
    ...(value.code !== undefined ? { code: value.code } : {}),
    ...(value.items !== undefined ? { items: value.items as number } : {}),
  };
}

const KINDS: readonly ItemKind[] = ["login", "passkey", "totp", "note", "wifi"];

export function summaryFrom(value: unknown): ItemSummary | null {
  if (!isRecord(value)) return null;
  const type = KINDS.find((kind) => kind === value.type);
  const { itemId, title, username, domains, hasCode, modifiedAt } = value;
  if (!type || !isString(itemId) || itemId === "" || !isString(title) || !isString(modifiedAt)) return null;
  if (username !== undefined && !isString(username)) return null;
  if (!strings(domains)) return null;
  if (hasCode !== undefined && typeof hasCode !== "boolean") return null;
  return {
    itemId,
    type,
    title,
    ...(username !== undefined ? { username } : {}),
    domains: [...domains],
    hasCode: hasCode === true,
    modifiedAt,
  };
}

export function itemFrom(value: unknown): ItemView | null {
  if (!isRecord(value)) return null;
  const { itemId, createdAt, modifiedAt } = value;
  if (!isString(itemId) || itemId === "" || !isString(createdAt) || !isString(modifiedAt)) return null;
  const base = { itemId, createdAt, modifiedAt };
  switch (value.type) {
    case "login": {
      const { title, username, password, urls, apps, totp, notes } = value;
      if (!isString(title) || !isString(username) || !isString(password)) return null;
      if (!strings(urls ?? []) || !strings(apps ?? []) || !optionalString(totp) || !optionalString(notes)) return null;
      return {
        ...base,
        type: "login",
        title,
        username,
        password,
        urls: [...((urls as string[] | undefined) ?? [])],
        apps: [...((apps as string[] | undefined) ?? [])],
        totp: totp ?? null,
        notes: notes ?? null,
      };
    }
    case "passkey": {
      const { rpId, rpName, userName, userDisplayName } = value;
      if (!isString(rpId) || !isString(rpName) || !isString(userName) || !isString(userDisplayName)) return null;
      return { ...base, type: "passkey", rpId, rpName, userName, userDisplayName };
    }
    case "totp": {
      const { title, issuer, account, uri, notes } = value;
      if (!isString(title) || !isString(uri) || !optionalString(issuer) || !optionalString(account)) return null;
      if (!optionalString(notes)) return null;
      return { ...base, type: "totp", title, issuer: issuer ?? null, account: account ?? null, uri, notes: notes ?? null };
    }
    case "note": {
      const { title, body } = value;
      if (!isString(title) || !isString(body)) return null;
      return { ...base, type: "note", title, body };
    }
    case "wifi": {
      const { ssid, password, security, hidden } = value;
      if (!isString(ssid) || !optionalString(password) || !isString(security) || typeof hidden !== "boolean") {
        return null;
      }
      return { ...base, type: "wifi", ssid, password: password ?? null, security, hidden };
    }
    default:
      return null;
  }
}

export function enrollmentFrom(value: unknown): Enrollment | null {
  if (!isRecord(value)) return null;
  const { enrollmentId, status, code, name, platform, expiresAt } = value;
  if (!isString(enrollmentId) || enrollmentId === "") return null;
  if (status !== "waiting" && status !== "code" && status !== "approved") return null;
  if (code !== undefined && !(isString(code) && /^\d{6}$/.test(code))) return null;
  if (status === "code" && code === undefined) return null;
  if (![name, platform, expiresAt].every((text) => text === undefined || isString(text))) return null;
  return {
    enrollmentId,
    status,
    ...(code !== undefined ? { code: code as string } : {}),
    ...(name !== undefined ? { name: name as string } : {}),
    ...(platform !== undefined ? { platform: platform as string } : {}),
    ...(expiresAt !== undefined ? { expiresAt: expiresAt as string } : {}),
  };
}

export function memberFrom(value: unknown): Member | null {
  if (!isRecord(value)) return null;
  const { memberId, name, platform, orphaned } = value;
  if (!isString(memberId) || memberId === "" || !isString(name) || !isString(platform)) return null;
  if (typeof value.self !== "boolean" || typeof orphaned !== "boolean") return null;
  return { memberId, name, platform, self: value.self, orphaned };
}

export function auditFrom(value: unknown): Audit | null {
  if (!isRecord(value)) return null;
  const { reusedCount, weakCount, reused, weak } = value;
  if (!Number.isInteger(reusedCount) || !Number.isInteger(weakCount)) return null;
  if (!Array.isArray(reused) || !reused.every(strings) || !strings(weak)) return null;
  return {
    reusedCount: reusedCount as number,
    weakCount: weakCount as number,
    reused: reused.map((group) => [...group]),
    weak: [...weak],
  };
}

export function codeFrom(value: unknown): Code | null {
  if (!isRecord(value)) return null;
  const { code, period, remaining } = value;
  if (!isString(code) || !/^\d{6,8}$/.test(code)) return null;
  if (!Number.isInteger(period) || (period as number) < 1) return null;
  if (!Number.isInteger(remaining) || (remaining as number) < 1 || (remaining as number) > (period as number)) {
    return null;
  }
  return { code, period: period as number, remaining: remaining as number };
}

const list = <T>(read: (value: unknown) => T | null) => (value: unknown): T[] | null => {
  if (!Array.isArray(value)) return null;
  const kept: T[] = [];
  for (const entry of value) {
    const one = read(entry);
    if (one === null) return null;
    kept.push(one);
  }
  return kept;
};

const unit = (value: unknown) => (value === null ? true : null);

/** The keychain as Settings uses it: each request, its answer read strictly. */
export function keychainClient(platform: KeychainPlatform) {
  const ask = async <T>(request: KeychainRequest, key: string, read: (value: unknown) => T | null) =>
    answered(await platform.ask(request), key, read);
  return {
    status: () => ask({ keychainStatus: {} }, "keychainStatus", statusFrom),
    verify: (password: string) => ask({ keychainVerify: { password } }, "keychainVerified", unit),
    /** The vault made, its recovery key shown once: the key, until it is typed back. */
    create: () =>
      ask({ keychainCreate: {} }, "keychainRecoveryKey", (value) =>
        isRecord(value) && isString(value.recoveryKey) && value.recoveryKey !== "" ? value.recoveryKey : null,
      ),
    confirmRecoveryKey: (recoveryKey: string) =>
      ask({ keychainConfirmRecoveryKey: { recoveryKey } }, "keychainStatus", statusFrom),
    joinByApproval: () => ask({ keychainJoin: { method: "approval" } }, "keychainEnrollment", enrollmentFrom),
    joinWithRecoveryKey: (recoveryKey: string) =>
      ask({ keychainJoin: { method: "recoveryKey", recoveryKey } }, "keychainStatus", statusFrom),
    enrollments: () => ask({ keychainEnrollments: {} }, "keychainEnrollments", list(enrollmentFrom)),
    /** Starts approving (the code comes once the device revealed its own), or follows it. */
    approve: (enrollmentId: string) => ask({ keychainApprove: { enrollmentId } }, "keychainEnrollment", enrollmentFrom),
    /** Approves, once the person confirmed the codes match. */
    confirmApproval: (enrollmentId: string) =>
      ask({ keychainApprove: { enrollmentId, confirm: true } }, "keychainApproved", unit),
    deny: (enrollmentId: string) => ask({ keychainDeny: { enrollmentId } }, "keychainDenied", unit),
    members: () => ask({ keychainMembers: {} }, "keychainMembers", list(memberFrom)),
    removeMember: (memberId: string) =>
      ask({ keychainRemoveMember: { memberId } }, "keychainMemberRemoved", unit),
    list: () => ask({ keychainList: {} }, "keychainItems", list(summaryFrom)),
    reveal: (itemId: string) => ask({ keychainReveal: { itemId } }, "keychainItem", itemFrom),
    put: (item: ItemInput) =>
      ask({ keychainPut: { item } }, "keychainSaved", (value) =>
        isRecord(value) && isString(value.itemId) && value.itemId !== "" ? value.itemId : null,
      ),
    remove: (itemId: string) => ask({ keychainDelete: { itemId } }, "keychainDeleted", unit),
    audit: () => ask({ keychainAudit: {} }, "keychainAudit", auditFrom),
    rekey: (previous: string, current: string) =>
      ask({ keychainRekey: { previous, current } }, "keychainStatus", statusFrom),
    leave: () => ask({ keychainLeave: {} }, "keychainLeft", unit),
    generate: () =>
      ask({ keychainGenerate: {} }, "generatedPassword", (value) => (isString(value) && value !== "" ? value : null)),
    code: (itemId: string) => ask({ keychainCode: { itemId } }, "keychainCode", codeFrom),
    copySensitive: (text: string) => platform.copySensitive(text),
    watchActive: platform.watchActive,
  };
}

export type KeychainClient = ReturnType<typeof keychainClient>;

/** The page's sidebar: the kinds of item, the warnings, the devices. */
export type Category = "all" | "passwords" | "passkeys" | "codes" | "wifi" | "notes" | "security" | "devices";

export const CATEGORIES: readonly Category[] = [
  "all",
  "passwords",
  "passkeys",
  "codes",
  "wifi",
  "notes",
  "security",
  "devices",
];

/** The items a category lists (none for the devices'); Security's are those with a warning. */
export function itemsIn(category: Category, items: readonly ItemSummary[], audit: Audit | null): ItemSummary[] {
  switch (category) {
    case "all":
      return [...items];
    case "passwords":
      return items.filter((item) => item.type === "login");
    case "passkeys":
      return items.filter((item) => item.type === "passkey");
    case "codes":
      return items.filter((item) => item.hasCode);
    case "wifi":
      return items.filter((item) => item.type === "wifi");
    case "notes":
      return items.filter((item) => item.type === "note");
    case "security": {
      const warned = new Set([...(audit?.weak ?? []), ...(audit?.reused.flat() ?? [])]);
      return items.filter((item) => warned.has(item.itemId));
    }
    case "devices":
      return [];
  }
}

/** The items whose title, username or sites hold what is searched, ignoring case and accents. */
export function search(items: readonly ItemSummary[], query: string): ItemSummary[] {
  const fold = (text: string) => text.normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase();
  const wanted = fold(query.trim());
  if (wanted === "") return [...items];
  return items.filter((item) =>
    [item.title, item.username ?? "", ...item.domains].some((text) => fold(text).includes(wanted)),
  );
}

/** An item's warnings: its password used by other items too (how many in all), and weak. */
export type Warnings = { reusedWith: number; weak: boolean };

export function warningsOf(itemId: string, audit: Audit | null): Warnings {
  const group = audit?.reused.find((ids) => ids.includes(itemId));
  return { reusedWith: group ? group.length : 0, weak: audit?.weak.includes(itemId) ?? false };
}

/** The letter an item without a site icon shows: its title's (or its site's) first. */
export function initialOf(item: Pick<ItemSummary, "title" | "domains">): string {
  const source = item.title.trim() || item.domains[0] || "";
  const first = [...source][0] ?? "";
  return first.toLocaleUpperCase();
}

/** A recovery key in its eight groups of four, as the service shows it ("ABCD-EFGH-…"). */
export function recoveryKeyGroups(key: string): string[] {
  const symbols = key.replace(/[\s-]/g, "").toUpperCase();
  const groups: string[] = [];
  for (let at = 0; at < symbols.length; at += 4) groups.push(symbols.slice(at, at + 4));
  return groups;
}

/** A verification code as people read it: "123 456" (four and four for eight digits). */
export function formatCode(code: string): string {
  const half = Math.ceil(code.length / 2);
  return `${code.slice(0, half)} ${code.slice(half)}`;
}

/**
 * What the person typed to set up a verification code: an `otpauth://totp/` URI as given, or a
 * secret (base32, as sites show it, spaces and case ignored) made into one for this item's
 * title and account. The service reads it (`keychainPut` refuses one it cannot): null when it is
 * neither.
 */
export function otpauthFrom(typed: string, title: string, account: string): string | null {
  const text = typed.trim();
  if (text === "") return null;
  if (/^otpauth:\/\/totp\//i.test(text)) return text;
  const secret = text.replace(/[\s-]/g, "").toUpperCase().replace(/=+$/, "");
  if (secret.length < 16 || !/^[A-Z2-7]+$/.test(secret)) return null;
  const issuer = title.trim();
  const label = issuer ? `${encodeURIComponent(issuer)}:${encodeURIComponent(account.trim())}` : encodeURIComponent(account.trim());
  return `otpauth://totp/${label}?secret=${secret}${issuer ? `&issuer=${encodeURIComponent(issuer)}` : ""}`;
}

/** A login's sites as an editor shows them, one a line, and back: blank lines dropped. */
export function urlsFromText(text: string): string[] {
  return text
    .split(/\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "")
    .map((line) => (/^[a-z][a-z0-9+.-]*:\/\//i.test(line) ? line : `https://${line}`));
}

/** What the page shows for the keychain's state. */
export type Stage =
  | "unavailable"
  | "locked"
  | "needsPreviousPassword"
  | "gate"
  | "turnOn"
  | "creating"
  | "join"
  | "joining"
  | "removed"
  | "unbound"
  | "open";

/**
 * The stage the page is at, from the service's status. Opening Passwords asks for the account's
 * password first, whatever the keychain's state (decision 8, as a Mac's Passwords asks for Touch ID
 * before anything): only a locked keychain (nothing can be verified) and one sealed under a
 * password the machine no longer has (its form asks for both passwords) come before it. A vault
 * not known at Oxy yet (`vault` null) is offered to be turned on: making it asks Oxy first, and
 * one that exists is joined instead.
 */
export function stageOf(status: KeychainStatus | null): Stage {
  if (!status) return "unavailable";
  if (status.state === "locked") return "locked";
  if (status.state === "needsPreviousPassword") return "needsPreviousPassword";
  if (!status.verified) return "gate";
  switch (status.state) {
    case "off":
      return status.vault === true ? "join" : "turnOn";
    case "creating":
      return "creating";
    case "joining":
      return "joining";
    case "removed":
      return "removed";
    case "unbound":
      return "unbound";
    case "member":
      return "open";
  }
}

/** Whether a request is the person's doing, which keeps Passwords open: not what the page asks on
    its own (the status, a code as it runs out, a join or an approval followed, the devices). */
export function personsRequest(request: KeychainRequest): boolean {
  if ("keychainStatus" in request || "keychainCode" in request) return false;
  if ("keychainEnrollments" in request || "keychainMembers" in request) return false;
  if ("keychainJoin" in request) return request.keychainJoin.method === "recoveryKey";
  if ("keychainApprove" in request) return request.keychainApprove.confirm === true;
  return true;
}

/** Whether what the page shows may stay, as the service tells the keychain now: unlocked and the
    person verified. Anything else (locked with the session, the proof gone with the lock) makes
    the page anew and asks for the password. */
export function staysOpen(status: KeychainStatus | null): boolean {
  return status !== null && status.state !== "locked" && status.state !== "needsPreviousPassword" && status.verified;
}

/** Whether a refusal means the page must be opened again: the proof lapsed, or the session
    locked. */
export function closesAccess(outcome: Outcome<unknown>): boolean {
  return "refused" in outcome && (outcome.refused === "verificationRequired" || outcome.refused === "locked");
}

/** What the person is told of a request the service refused or could not answer. */
export function outcomeMessage(outcome: Exclude<Outcome<unknown>, { ok: unknown }>, words: PasswordsMessages): string {
  if ("failed" in outcome) return words.failed;
  switch (outcome.refused) {
    case "wrongPassword":
      return words.wrongPassword;
    case "throttled":
      return outcome.retryAfter ? words.tryAgainIn(outcome.retryAfter) : words.tryAgainLater;
    case "offline":
    case "oxy":
      return words.offline;
    case "recoveryKeyMismatch":
      return words.recoveryMismatch;
    case "denied":
      return words.denied;
    case "expired":
      return words.expired;
    case "removed":
      return words.removed;
    case "conflict":
      return words.conflict;
    case "notFound":
      return words.notFound;
    default:
      return words.failed;
  }
}
