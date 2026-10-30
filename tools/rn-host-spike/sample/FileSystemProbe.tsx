/**
 * expo-file-system and the Bloom adapter's places and Trash on OxyOS (sample/oxyos/expo-file-system.js,
 * docs/adr/0028-file-access.mdx), for scenarios/file-system.json: the real package's `File`,
 * `Directory` and `Paths` (and `expo-file-system/legacy`'s upload), and Bloom's `usePlatformFiles`,
 * as an application uses them, on a home the scenario makes. Each line says what the application
 * got: a value, or an error's code.
 *
 * - On start: the application's folders, reading and writing, files and folders made, listed,
 *   copied, moved, renamed and deleted, an open file, digests, what the scope refuses and what
 *   OxyOS cannot do yet.
 * - Trash: the places, items put in the Trash, one put back, one erased, the Trash's watch.
 * - Watch: a folder watched while another program drops a file in it, which the application then
 *   renames.
 * - Network: a download into a folder and its progress, uploads (the file as the body, the legacy
 *   upload task, a multipart form with the file), and a failed download's status.
 */
import * as React from 'react';
import { View } from 'react-native';
import { Directory, File, Paths } from 'expo-file-system';
import * as FileSystemLegacy from 'expo-file-system/legacy';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { usePlatformFiles } from '@oxy.so/bloom/platform-surface';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

const SERVER = 'http://127.0.0.1:18652';

type Line = [string, string];
type Say = (name: string, value: unknown) => void;

/** What a step gave, or the code of what it threw. */
async function attempt(say: Say, name: string, step: () => unknown) {
  try {
    say(name, await step());
  } catch (error) {
    const coded = error as { code?: string; message?: string };
    say(name, `threw ${coded.code ?? coded.message}`);
  }
}

/** The part of a URI after the folder `marker` names, to show without the run's own paths. */
const after = (uri: string, marker: string) => uri.slice(uri.indexOf(marker));

async function files(say: Say) {
  await attempt(say, 'document', () => after(Paths.document.uri, '/.local/share/'));
  await attempt(say, 'cache', () => after(Paths.cache.uri, '/.cache/'));
  await attempt(say, 'bundle', () => new File(Paths.bundle, 'index.js').exists);
  await attempt(say, 'legacy document', () => FileSystemLegacy.documentDirectory === Paths.document.uri);

  const notes = new File(Paths.document, 'notes.txt');
  await attempt(say, 'create', async () => {
    notes.create();
    await notes.write('Hola, ');
    notes.writeSync('món', { append: true });
    return notes.textSync();
  });
  await attempt(say, 'create again', () => notes.create());
  await attempt(say, 'text', () => notes.text());
  await attempt(say, 'base64', () => notes.base64Sync());
  await attempt(say, 'bytes', async () => (await notes.bytes()).length);
  await attempt(say, 'facts', () => `${notes.exists} ${notes.size} ${notes.type} ${notes.lastModified! > 0}`);
  await attempt(say, 'md5', () => notes.md5);
  await attempt(say, 'sha256', () => notes.digest('SHA-256'));
  await attempt(say, 'info', () => {
    const info = notes.info();
    return `${info.exists} ${info.size}`;
  });

  const projects = new Directory(Paths.document, 'Projects');
  await attempt(say, 'folders', () => {
    projects.create();
    projects.createFile('plan.md', 'text/markdown').writeSync('# Plan');
    projects.createDirectory('Sub');
    return projects
      .list()
      .map((item) => (item instanceof Directory ? `${item.name}/` : item.name))
      .join(' ');
  });
  await attempt(say, 'copy', async () => {
    notes.copy(projects);
    await notes.copy(new File(Paths.document, 'copy.txt'));
    return new File(projects, 'notes.txt').textSync();
  });
  await attempt(say, 'copy over', () => notes.copySync(new File(Paths.document, 'copy.txt')));
  await attempt(say, 'copy overwrite', () => {
    notes.copySync(new File(Paths.document, 'copy.txt'), { overwrite: true });
    return 'ok';
  });
  const moved = new File(Paths.document, 'copy.txt');
  await attempt(say, 'move', async () => {
    await moved.move(new Directory(projects, 'Sub'));
    return after(moved.uri, 'Projects/');
  });
  await attempt(say, 'rename', () => {
    moved.rename('renamed.txt');
    return after(moved.uri, 'Projects/');
  });
  await attempt(say, 'directory info', () => {
    const info = projects.info();
    return `${info.files?.join(' ')} ${info.size}`;
  });
  await attempt(say, 'delete', () => {
    const sub = new Directory(projects, 'Sub');
    sub.delete();
    return `${sub.exists} ${moved.exists}`;
  });
  await attempt(say, 'delete again', () => new Directory(projects, 'Sub').delete());

  const data = new File(Paths.document, 'data.bin');
  await attempt(say, 'handle', async () => {
    const handle = data.open();
    handle.writeBytesSync(new Uint8Array([1, 2, 3, 4, 5]));
    handle.offset = 1;
    const read = await handle.readBytes(2);
    const seen = `${[...read].join(',')} at ${handle.offset} of ${handle.size}`;
    handle.close();
    return `${seen}, closed ${handle.offset}`;
  });

  await attempt(say, 'home file', () => new File(Paths.document.uri.replace(/\.local\/share\/.*$/, ''), 'Documents', 'report.txt').textSync());
  await attempt(say, 'outside', () => new File('file:///etc/hostname').textSync());
  await attempt(say, 'outside exists', () => new File('file:///etc/hostname').exists);
  await attempt(say, 'bundle delete', () => new File(Paths.bundle, 'index.js').delete());
  await attempt(say, 'documents delete', () => Paths.document.delete());
  await attempt(say, 'space', () => Paths.availableDiskSpace > 0 && Paths.totalDiskSpace >= Paths.availableDiskSpace);
  await attempt(say, 'pick folder', () => Directory.pickDirectoryAsync());
  await attempt(say, 'can preview', () => notes.canPreview());
  await attempt(say, 'preview', () => notes.preview());
  await attempt(say, 'legacy read', () => FileSystemLegacy.readAsStringAsync(notes.uri));
}

type Files = NonNullable<ReturnType<typeof usePlatformFiles>>;

async function trash(say: Say, platform: Files, changes: () => number) {
  const places = await platform.places();
  say('places', places.map(({ kind, name }) => `${kind}:${name}`).join(' '));
  const home = places.find(({ kind }) => kind === 'home')!.uri;
  const documents = new Directory(home, 'Documents');
  const outcomes = await platform.trash([
    new File(documents, 'old.txt').uri,
    new File(documents, 'draft.txt').uri,
    new File(documents, 'junk.txt').uri,
    home,
    'file:///etc/hostname',
  ]);
  say('trashed', outcomes.map((outcome) => (outcome.ok ? 'ok' : outcome.error)).join(' '));
  const held = await platform.trashed();
  say(
    'trash holds',
    held
      .map(({ name, originalUri }) => `${name} from ${after(originalUri ?? '', 'Documents/')}`)
      .sort()
      .join(', '),
  );
  const uriOf = (name: string) => held.find((item) => item.name === name)!.uri;
  const restored = await platform.restore([uriOf('draft.txt')]);
  say('restored', restored.map((outcome) => (outcome.ok ? after(outcome.result, 'Documents/') : outcome.error)).join(' '));
  const erased = await platform.erase([uriOf('junk.txt'), new File(documents, 'draft.txt').uri]);
  say('erased', erased.map((outcome) => (outcome.ok ? 'ok' : outcome.error)).join(' '));
  say('draft back', new File(documents, 'draft.txt').textSync());
  // The Trash's watch heard it change.
  await new Promise((resolve) => setTimeout(resolve, 500));
  say('trash heard', changes() > 0);
}

async function network(say: Say) {
  const progress: string[] = [];
  await attempt(say, 'download', async () => {
    const file = await File.downloadFileAsync(`${SERVER}/report`, Paths.document, {
      onProgress: ({ bytesWritten, totalBytes }) => progress.push(`${bytesWritten}/${totalBytes}`),
    });
    return `${file.name} ${file.textSync()} ${progress[progress.length - 1]}`;
  });
  await attempt(say, 'download again', () => File.downloadFileAsync(`${SERVER}/report`, Paths.document));
  await attempt(say, 'download missing', () =>
    File.downloadFileAsync(`${SERVER}/missing`, new File(Paths.document, 'missing.txt')).catch((error) => {
      throw { code: `${error.code} (${error.message})` };
    }),
  );
  const notes = new File(Paths.document, 'notes.txt');
  await attempt(say, 'upload', async () => {
    const sent: number[] = [];
    const result = await notes.upload(`${SERVER}/upload`, {
      httpMethod: 'PUT',
      onProgress: ({ bytesSent }) => sent.push(bytesSent),
    });
    return `${result.status} ${result.body} sent ${sent[sent.length - 1]}`;
  });
  await attempt(say, 'legacy upload', async () => {
    let sent = 0;
    const task = FileSystemLegacy.createUploadTask(
      `${SERVER}/upload`,
      notes.uri,
      {
        httpMethod: 'PUT',
        uploadType: FileSystemLegacy.FileSystemUploadType.BINARY_CONTENT,
        sessionType: FileSystemLegacy.FileSystemSessionType.FOREGROUND,
        headers: {},
      },
      ({ totalBytesSent }) => {
        sent = totalBytesSent;
      },
    );
    const result = await task.uploadAsync();
    return `${result?.status} ${result?.body} sent ${sent}`;
  });
  await attempt(say, 'form with file', async () => {
    const form = new FormData();
    form.append('owner', 'ada');
    form.append('document', { uri: notes.uri, name: 'notes.txt', type: 'text/plain' } as never);
    return (await fetch(`${SERVER}/echo`, { method: 'POST', body: form })).text();
  });
  await attempt(say, 'form outside', async () => {
    const form = new FormData();
    form.append('document', { uri: 'file:///etc/hostname', name: 'hostname', type: 'text/plain' } as never);
    return (await fetch(`${SERVER}/echo`, { method: 'POST', body: form })).text();
  });
}

function Probe() {
  const platform = usePlatformFiles();
  const [lines, setLines] = React.useState<Line[]>([]);
  const say = React.useCallback<Say>((name, value) => {
    setLines((shown) => [...shown, [name, typeof value === 'string' ? value : JSON.stringify(value)]]);
  }, []);
  const trashChanges = React.useRef(0);
  React.useEffect(() => {
    files(say).then(() => say('files', 'done'));
    return platform?.watchTrash(() => {
      trashChanges.current += 1;
    });
  }, [platform, say]);
  const watch = () => {
    const inbox = new Directory(Paths.document.uri.replace(/\.local\/share\/.*$/, ''), 'Inbox');
    let seen = 0;
    inbox.watch(
      (event) => {
        seen += 1;
        const to = event.newTarget ? ` to ${event.newTarget.name}` : '';
        say(`event ${seen}`, `${event.type} ${event.target.name}${to}`);
        // What another program dropped, the application renames: the watch says that too.
        if (event.type === 'created' && event.target.name === 'drop.txt') {
          (event.target as File).rename('kept.txt');
        }
      },
      { debounce: 100 },
    );
    say('watch', 'watching Inbox');
  };
  return (
    <View style={{ padding: 16, gap: 4 }}>
      <View style={{ flexDirection: 'row', gap: 12, marginBottom: 8 }}>
        <Button
          testID="trash"
          onPress={() => trash(say, platform!, () => trashChanges.current).catch((error) => say('trash failed', error.message))}
        >
          Trash
        </Button>
        <Button testID="watch" onPress={watch}>
          Watch
        </Button>
        <Button testID="network" onPress={() => network(say).then(() => say('network', 'done'))}>
          Network
        </Button>
      </View>
      <View style={{ flexDirection: 'row', flexWrap: 'wrap', columnGap: 24 }}>
        {lines.map(([name, value], index) => (
          <Muted key={index} style={{ width: 460, fontSize: 12, lineHeight: 16 }}>
            {`${name}: ${value}`}
          </Muted>
        ))}
      </View>
    </View>
  );
}

export default function FileSystemProbe() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <Probe />
    </BloomProvider>
  );
}
