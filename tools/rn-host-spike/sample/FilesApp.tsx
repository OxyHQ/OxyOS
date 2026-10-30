/**
 * OxyOS's Files (apps/files) in the Bloom host (docs/adr/0027-files.mdx). Its file system is the
 * host's `OxyOSFiles`: the user's home and the volumes mounted for them, through
 * crates/oxyos-files, each request answered on the service's own thread (`OxyOSFilesAnswer`) and
 * each watched change said (`OxyOSFilesChanged`). Without the module (a host run outside an
 * application's window, the browser preview) the model is a folder tree kept in memory.
 */
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { Files } from '../../../apps/files/src/Files';
import { MemoryFileSystem } from '../../../apps/files/src/memory';
import {
  FileError,
  type FileErrorCode,
  type FileSystem,
  type ItemResult,
  type TrashedEntry,
  type WatchTarget,
} from '../../../apps/files/src/model';
import { stringsFor } from '../../../apps/files/src/strings';

const native = TurboModuleRegistry.get<{ request(json: string): void }>('OxyOSFiles');

interface Answer {
  id: number;
  ok: boolean;
  value?: unknown;
  error?: FileErrorCode;
  message?: string;
}

/** One result of a request about several items, as the service writes it (oxyos_files::service):
 * the item under `path` or `id`, and what came of it under another key, or its error. */
type Each = Record<string, unknown> & { ok: boolean; error?: FileErrorCode };

function results(
  items: Each[],
  item: 'path' | 'id',
  outcome: 'to' | 'id' | 'path' | 'removed',
): ItemResult[] {
  return items.map((each) => {
    const name = String(each[item] ?? '');
    return each.ok
      ? { item: name, ok: true as const, to: String(each[outcome] ?? name) }
      : { item: name, ok: false as const, error: each.error ?? 'io' };
  });
}

/** The host's file service as the application's `FileSystem`. */
function hostFileSystem(module: { request(json: string): void }): FileSystem {
  let next = 0;
  const pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (error: FileError) => void }
  >();
  const watchers = new Map<number, () => void>();
  DeviceEventEmitter.addListener('OxyOSFilesAnswer', (json: string) => {
    const answer = JSON.parse(json) as Answer;
    const waiting = pending.get(answer.id);
    if (!waiting) return;
    pending.delete(answer.id);
    if (answer.ok) waiting.resolve(answer.value);
    else waiting.reject(new FileError(answer.error ?? 'io', answer.message ?? ''));
  });
  DeviceEventEmitter.addListener('OxyOSFilesChanged', (json: string) => {
    const { changed } = JSON.parse(json) as { changed: number };
    watchers.get(changed)?.();
  });
  const ask = <T,>(op: string, args: Record<string, unknown> = {}): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      next += 1;
      pending.set(next, { resolve: resolve as (value: unknown) => void, reject });
      module.request(JSON.stringify({ id: next, op, ...args }));
    });
  return {
    places: () => ask('places'),
    list: (path) => ask('list', { path }),
    createFolder: (parent, name) => ask('createFolder', { parent, name }),
    rename: (path, name) => ask('rename', { path, name }),
    move: async (paths, to) => results(await ask<Each[]>('move', { paths, to }), 'path', 'to'),
    trash: async (paths) => results(await ask<Each[]>('trash', { paths }), 'path', 'id'),
    // An item in the Trash is named by its id there, which is its path in the Trash too.
    trashList: () => ask<TrashedEntry[]>('trashList'),
    restore: async (ids) => results(await ask<Each[]>('restore', { ids }), 'id', 'path'),
    deleteFromTrash: async (ids) => results(await ask<Each[]>('deleteFromTrash', { ids }), 'id', 'removed'),
    emptyTrash: () => ask('emptyTrash'),
    watch: async (target: WatchTarget, changed) => {
      const watch = await ask<number>('watch', target as Record<string, unknown>);
      watchers.set(watch, changed);
      return () => {
        watchers.delete(watch);
        ask('unwatch', { watch }).catch(() => {});
      };
    },
  };
}

/** A home to look at where there is no file service. */
function fixture(): FileSystem {
  const day = 24 * 60 * 60 * 1000;
  const now = Date.now();
  return new MemoryFileSystem(
    '/home/ada',
    {
      'Desktop/': null,
      'Documents/': null,
      'Documents/Report 2026.pdf': { size: 482_133, modified: now - 2 * day },
      'Documents/notes.md': { size: 1_204, modified: now - 3_600_000 },
      'Documents/Budget.xlsx': { size: 38_912, modified: now - 40 * day },
      'Downloads/': null,
      'Downloads/oxyos.iso': { size: 1_840_000_000, modified: now - 6 * day },
      'Pictures/': null,
      'Pictures/Beach.jpg': { size: 2_400_000, modified: now - 300 * day },
      'Music/': null,
      'Videos/': null,
      'Projects/': null,
      'Projects/oxyos/': null,
      '.bashrc': { size: 3_771, modified: now - 900 * day },
    },
    [{ name: 'USB Stick', path: '/media/ada/USB Stick' }],
  );
}

export default function FilesApp() {
  const fs = React.useMemo(() => (native ? hostFileSystem(native) : fixture()), []);
  const locale = React.useMemo(() => Intl.DateTimeFormat().resolvedOptions().locale, []);
  const strings = React.useMemo(() => stringsFor(locale), [locale]);
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces}>
        <Files fs={fs} strings={strings} locale={locale} />
      </BloomProvider>
    </SafeAreaProvider>
  );
}
