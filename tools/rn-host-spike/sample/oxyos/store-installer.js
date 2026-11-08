// The store's installer on OxyOS (docs/adr/0030-installing-applications.mdx, decision 9): what
// @oxy.so/core's `StoreInstaller` asks (oxy ADR 0053), answered by the session through
// `OxyOSSystem` (oxyos/system.js) with the `store` and `updates` grants. The runtime registers it
// in the application's own Oxy SDK (oxyos/store-registration.js), for an application the image's
// policy gives `store` alone: Atlas. It is the store's hands, never its judge: it names a listing
// by its slug, the install service resolves it from the store's signed metadata, the shell asks
// the person in the system's sheet, and the session decides. Nothing here installs, verifies or
// grants anything.
import { application, settings, store } from '@oxyos/platform';

/** The person's language, for the store's texts the machine keeps in several. */
function language() {
  try {
    return Intl.DateTimeFormat().resolvedOptions().locale.split('-')[0];
  } catch {
    return 'en';
  }
}

/** A text in the person's language, else English, else any; `fallback` without one. */
function localized(texts, fallback = null) {
  if (!texts) return fallback;
  return texts[language()] ?? texts.en ?? Object.values(texts)[0] ?? fallback;
}

/** The session's update state as the store reads it (@oxy.so/core's `StoreUpdates`). */
function storeUpdates(state) {
  if (!state) return null;
  return {
    updates: state.updates.map((update) => ({
      id: update.id,
      name: localized(update.names, update.name),
      version: update.fromVersion,
      newVersion: update.toVersion,
      downloadSize: update.download,
      releaseNotes: localized(update.releaseNotes),
      needsConsent: update.status === 'awaitingConsent',
      atNextLaunch: update.status === 'atNextLaunch',
    })),
    checking: state.checking,
    lastChecked: state.checkedAt ?? null,
    checkFailed: Boolean(state.checkFailed),
  };
}

/**
 * The installer, built with the application's own SDK's error class (so its `instanceof` holds),
 * or null for a launch without `store`.
 */
export function storeInstaller(StoreInstallerError) {
  const granted = application.grants;
  if (!granted.includes('store')) return null;

  /** What a session's answer that is not the one asked for means to the store. */
  // The platform API's error (`@oxyos/platform`'s PlatformError) as the SDK's: whether the session
  // refused it or tried it and failed, its own reason, and whether it already said so in its sheet.
  function failure(error) {
    const details = error?.details ?? {};
    const told = details.told === true;
    const message = details.detail ?? error?.reason ?? error?.code ?? 'no answer';
    const reason =
      !error?.refused && error?.reason === 'unavailable'
        ? 'serviceUnavailable'
        : error?.reason === 'notInstalled'
          ? 'notInstalled'
          : error?.reason === 'noRelease'
            ? 'notAvailable'
            : error?.refused
              ? 'refused'
              : 'failed';
    return new StoreInstallerError(reason, { told, message });
  }
  /** A session request's answer, its rejection as the SDK's error. */
  const asked = (pending) => pending.catch((error) => {
    throw failure(error);
  });

  /** Each listing's package id, as the install service resolved it (null: none for this machine). */
  const packages = new Map();
  /** Listings being installed or updated, with how far along. */
  const installing = new Map();
  const listeners = new Set();

  function tell(slug, state) {
    for (const listener of listeners) listener(slug, state);
  }

  function stateOf(entry) {
    if (installing.has(entry.listing)) return { state: 'installing', fraction: installing.get(entry.listing) };
    // An install the session has under way that this store did not ask for, or asked for before
    // it was opened again: installing, as far along as the session last heard.
    if (entry.installing) return { state: 'installing', fraction: fractionOf(entry.installing.progress) };
    if (!entry.id) return { state: 'unavailable' };
    if (!entry.installed) return { state: 'notInstalled' };
    const update = entry.update;
    if (update && (update.status === 'available' || update.status === 'awaitingConsent')) {
      return {
        state: 'updateAvailable',
        version: entry.version,
        newVersion: update.toVersion,
        needsConsent: update.status === 'awaitingConsent',
      };
    }
    return { state: 'installed', version: entry.version };
  }

  async function status(slugs) {
    const answer = await asked(store.status({ listings: [...slugs] }));
    if (!Array.isArray(answer.status)) throw failure(null);
    const states = {};
    for (const entry of answer.status) {
      if (typeof entry.listing !== 'string') continue;
      packages.set(entry.listing, entry.id ?? null);
      states[entry.listing] = stateOf(entry);
    }
    return states;
  }

  /** Asks again where `slug` stands, and tells every listener. */
  async function refresh(slug) {
    try {
      const states = await status([slug]);
      if (states[slug]) tell(slug, states[slug]);
    } catch {
      // The listing's last state stays; the next status says where it stands.
    }
  }

  /** The package id of `slug`, resolving it when it was not yet. */
  async function packageOf(slug) {
    if (!packages.get(slug)) await status([slug]);
    const id = packages.get(slug);
    if (!id) throw new StoreInstallerError('notAvailable');
    return id;
  }

  /** How far along an install's progress is, 0 to 1; null when it cannot say (compiling). */
  function fractionOf(progress) {
    const fetching = progress && typeof progress === 'object' ? progress.fetching : null;
    return fetching && fetching.total > 0 ? Math.min(1, fetching.done / fetching.total) : null;
  }

  function progressed(slug, progress) {
    const fraction = fractionOf(progress);
    installing.set(slug, fraction);
    tell(slug, { state: 'installing', fraction });
  }

  // The account's updates changed: the listings the store knows may have updates now, or none.
  settings.updates.subscribe(() => {
    const known = [...packages.keys()].filter((slug) => !installing.has(slug));
    if (known.length === 0) return;
    status(known).then(
      (states) => Object.entries(states).forEach(([slug, state]) => tell(slug, state)),
      () => {},
    );
  });

  return {
    status,

    async install(slug) {
      // Asked by its listing: the session resolves it, or says in the sheet that the install
      // service is not there. The ring shows only once the person confirmed (the first progress).
      let answer;
      try {
        answer = await store.install({ listing: slug }, { onProgress: (progress) => progressed(slug, progress) });
      } catch (error) {
        installing.delete(slug);
        await refresh(slug);
        throw failure(error);
      }
      installing.delete(slug);
      await refresh(slug);
      if (answer.installed) return 'installed';
      if (answer.cancelled) return 'declined';
      throw failure(null);
    },

    async update(slug) {
      const id = await packageOf(slug);
      installing.set(slug, null);
      tell(slug, { state: 'installing', fraction: null });
      let answer;
      try {
        answer = await settings.update([id]);
      } catch (error) {
        installing.delete(slug);
        await refresh(slug);
        throw failure(error);
      }
      installing.delete(slug);
      await refresh(slug);
      if (!Array.isArray(answer.updated)) throw failure(null);
      const outcome = answer.updated.find((u) => u.id === id)?.outcome;
      if (outcome === 'awaitingConsent') return 'declined';
      if (outcome === 'failed') throw new StoreInstallerError('failed');
      return 'updated';
    },

    async uninstall(slug) {
      const id = await packageOf(slug);
      let answer;
      try {
        answer = await store.uninstall(id);
      } catch (error) {
        await refresh(slug);
        throw failure(error);
      }
      await refresh(slug);
      if (answer.uninstalled) return 'removed';
      if (answer.cancelled) return 'declined';
      throw failure(null);
    },

    async open(slug) {
      const id = await packageOf(slug);
      const answer = await asked(store.open(id));
      if (!answer.opened) throw failure(null);
    },

    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },

    updates: {
      current: () => storeUpdates(settings.updates.current()),
      async read() {
        const answer = await asked(settings.refreshUpdates());
        if (!answer.updateState) throw failure(null);
        return storeUpdates(answer.updateState);
      },
      async check() {
        const answer = await asked(settings.checkUpdates());
        if (!answer.updateState) throw failure(null);
        return storeUpdates(answer.updateState);
      },
      async apply(ids) {
        const answer = await asked(settings.update([...ids]));
        if (!Array.isArray(answer.updated)) throw failure(null);
      },
      async applyAll() {
        const answer = await asked(settings.updateAll());
        if (!Array.isArray(answer.updated)) throw failure(null);
      },
      subscribe(listener) {
        return settings.updates.subscribe((state) => listener(storeUpdates(state)));
      },
    },
  };
}
