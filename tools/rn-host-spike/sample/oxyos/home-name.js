// The home folder's name in the places the platform lists (`files.places()`): see `homeName`.

/** A folder's own name, from its URI. */
const named = (uri) => decodeURIComponent(uri.replace(/\/+$/, '').split('/').pop());

/**
 * The home's name, as a Mac's sidebar names the home by its short name: the Oxy account's username,
 * as the session's host keeps it (session-host.js: the accounts' service's snapshot, followed,
 * never asked here). Never the Unix user's (`oxy-…`, the folder's own name), an identifier no one
 * is shown (docs/adr/0020-oxy-account-login.mdx, decision 1); the folder's name only where no
 * account is signed in (a development session with no accounts' service).
 */
export function homeName(home, host = globalThis[Symbol.for('oxy.so/session-host')]) {
  const account = host?.snapshot?.()?.account;
  const handle = typeof account?.handle === 'string' ? account.handle.trim() : '';
  return handle || named(home);
}
