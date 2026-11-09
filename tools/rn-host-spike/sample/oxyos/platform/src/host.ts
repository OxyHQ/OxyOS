// A call of one of the host's modules, as every call of the platform API behaves
// (docs/adr/0058-the-platform-api.mdx, decision 3): a synchronous throw of the host's (no window,
// no connection to the compositor) a rejection with `unavailable`, never a throw; aborted, a
// rejection with `cancelled` at once, the host told so it stops what it can (`onAbort`), and its
// answer, when it comes, dropped.
import { PlatformError, type PlatformDomain } from './errors.ts';
import type { CallOptions } from './frameworks.ts';

export function hostCall<T>(
  domain: PlatformDomain,
  start: () => Promise<T>,
  options: CallOptions = {},
  onAbort?: () => void,
): Promise<T> {
  const { signal } = options;
  if (signal?.aborted) return Promise.reject(new PlatformError(domain, 'cancelled'));
  let pending: Promise<T>;
  try {
    pending = start();
  } catch {
    return Promise.reject(new PlatformError(domain, 'unavailable'));
  }
  if (!signal) return pending;
  return new Promise<T>((resolve, reject) => {
    const abort = () => {
      onAbort?.();
      reject(new PlatformError(domain, 'cancelled'));
    };
    signal.addEventListener('abort', abort, { once: true });
    pending.then(
      (value) => {
        signal.removeEventListener('abort', abort);
        resolve(value);
      },
      (error) => {
        signal.removeEventListener('abort', abort);
        reject(error);
      },
    );
  });
}
