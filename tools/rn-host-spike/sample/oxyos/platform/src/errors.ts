// How every call of the platform API fails (docs/adr/0058-the-platform-api.mdx, decision 3, "One
// error"): it rejects with a PlatformError, a domain (its framework) and a code from one closed
// list, never by resolving `{refused}` or `{failed}`, never by throwing synchronously, never by
// answering `false`.

/** Every code a call may fail with; each framework documents which it uses. */
export const PLATFORM_ERROR_CODES = [
  'notPermitted',
  'needsInput',
  'unavailable',
  'notFound',
  'exists',
  'invalid',
  'busy',
  'locked',
  'cancelled',
  'failed',
] as const;
export type PlatformErrorCode = (typeof PLATFORM_ERROR_CODES)[number];

/** The framework a call belongs to. */
export type PlatformDomain =
  | 'application'
  | 'audio'
  | 'bluetooth'
  | 'network'
  | 'power'
  | 'settings'
  | 'store';

export class PlatformError extends Error {
  /** The framework whose call failed. */
  readonly domain: PlatformDomain;
  readonly code: PlatformErrorCode;
  /** The service's own word for it, when it said one beyond the code (`wrongPassword`). */
  readonly reason: string | undefined;

  constructor(domain: PlatformDomain, code: PlatformErrorCode, reason?: string) {
    super(reason && reason !== code ? `${domain}: ${code} (${reason})` : `${domain}: ${code}`);
    this.name = 'PlatformError';
    this.domain = domain;
    this.code = code;
    this.reason = reason;
  }
}

/** Whether `error` is a PlatformError (of `code`, if given). */
export function isPlatformError(error: unknown, code?: PlatformErrorCode): error is PlatformError {
  return error instanceof PlatformError && (code === undefined || error.code === code);
}

const CODES = new Set<string>(PLATFORM_ERROR_CODES);
// The session's words that are a code by another name.
const SYNONYMS: Record<string, PlatformErrorCode> = {
  invalidRequest: 'invalid',
  // The request never reached the session (no connection to it).
  notSent: 'unavailable',
};

/** The code a session's reason is: itself where it is one, else `failed` (the reason kept). */
export function codeOf(reason: string | undefined): PlatformErrorCode {
  if (reason === undefined) return 'failed';
  if (CODES.has(reason)) return reason as PlatformErrorCode;
  return SYNONYMS[reason] ?? 'failed';
}

type Reasoned = { reason?: unknown } | null | undefined;
type Answer = { refused?: Reasoned; failed?: Reasoned } | null | undefined;

/**
 * The error a session's answer is, or null for one that succeeded: `{refused: {reason}}` and
 * `{failed: {reason}}` are rejections at the package's edge (ADR 0058, step 2).
 */
export function errorOf(domain: PlatformDomain, answer: unknown): PlatformError | null {
  if (answer === null || answer === undefined || typeof answer !== 'object') {
    return new PlatformError(domain, 'failed', 'noAnswer');
  }
  const told = answer as Answer;
  const refusal = told?.refused ?? told?.failed;
  if (refusal === undefined) return null;
  const reason = typeof refusal?.reason === 'string' ? refusal.reason : undefined;
  return new PlatformError(domain, codeOf(reason), reason);
}
