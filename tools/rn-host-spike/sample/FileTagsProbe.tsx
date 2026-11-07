/**
 * Tags on the person's files through the Bloom adapter (`files.tags`, sample/oxyos/platform-surface.js,
 * docs/adr/0049-file-tags.mdx), for scenarios/file-tags.json: Bloom's `usePlatformFiles().tags` as
 * a file manager uses it, beside expo-file-system's copies and moves, on a home the scenario makes
 * (whose items' attributes the scenario sets and checks). Each line says what the application got:
 * a value, or an error's code.
 *
 * - On start: the index's watch, and its tags' names (a Mac's file's).
 * - Tag: items tagged (one outside refused), a folder's tags, items' tags, a name refused.
 * - Mac: a Mac's tags read and changed (its attribute rewritten), a tag respelt in place.
 * - Copy and move: a tagged file copied and another moved, both keeping their tags; the index
 *   finding them.
 * - Rename and Trash: a tagged file renamed (the index follows), put in the Trash (it leaves the
 *   index, its tags in the Trash's list) and put back (it comes back); every tag's name.
 * - Another program: a file tagged by another program, heard through the watch and found.
 */
import * as React from 'react';
import { View } from 'react-native';
import { Directory, File } from 'expo-file-system';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { usePlatformFiles } from '@oxy.so/bloom/platform-surface';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

type Line = [string, string];
type Say = (name: string, value: unknown) => void;

// Bloom's `PlatformFileTags` (its `PlatformFiles.tags`), as the adapter gives it.
type TagColor = 'red' | 'orange' | 'yellow' | 'green' | 'blue' | 'purple' | 'grey';
interface FileTag {
  readonly name: string;
  readonly color: TagColor | null;
}
interface FileOutcome {
  readonly uri: string;
  readonly ok: boolean;
  readonly result?: string;
  readonly error?: string;
}
interface TaggedItem {
  readonly uri: string;
  readonly name: string;
  readonly isDirectory: boolean;
  readonly size: number | null;
  readonly lastModified: number | null;
  readonly tags: readonly FileTag[];
}
interface FileTags {
  inFolder(folderUri: string): Promise<Readonly<Record<string, readonly FileTag[]>>>;
  ofItems(uris: readonly string[]): Promise<readonly (readonly FileTag[] | null)[]>;
  change(
    uris: readonly string[],
    change: { add?: readonly FileTag[]; remove?: readonly string[] },
  ): Promise<readonly FileOutcome[]>;
  tagged(name: string): Promise<readonly TaggedItem[]>;
  names(): Promise<readonly { name: string; count: number }[]>;
  watch(listener: () => void): () => void;
}
type Files = NonNullable<ReturnType<typeof usePlatformFiles>>;
type TaggedFiles = Files & { readonly tags?: FileTags };
type TrashedWithTags = { name: string; tags?: readonly FileTag[] };

/** What a step gave, or the code of what it threw. */
async function attempt(say: Say, name: string, step: () => unknown) {
  try {
    say(name, await step());
  } catch (error) {
    const coded = error as { code?: string; message?: string };
    say(name, `threw ${coded.code ?? coded.message}`);
  }
}

const shown = (tags: readonly FileTag[] | null) =>
  tags === null ? 'null' : tags.map(({ name, color }) => (color ? `${name}(${color})` : name)).join(' ') || 'none';
const outcomes = (list: readonly FileOutcome[]) => list.map((item) => (item.ok ? 'ok' : item.error)).join(' ');
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** The probe's view of the platform: the home, its Documents, and the tags. */
function useTags() {
  const platform = usePlatformFiles() as TaggedFiles | undefined;
  const [home, setHome] = React.useState<string | null>(null);
  const heard = React.useRef(0);
  React.useEffect(() => {
    platform?.places().then((places) => setHome(places.find(({ kind }) => kind === 'home')!.uri));
    return platform?.tags?.watch(() => {
      heard.current += 1;
    });
  }, [platform]);
  return { platform, home, heard };
}

function Probe() {
  const { platform, home, heard } = useTags();
  const [lines, setLines] = React.useState<Line[]>([]);
  const say = React.useCallback<Say>((name, value) => {
    setLines((seen) => [...seen, [name, typeof value === 'string' ? value : JSON.stringify(value)]]);
  }, []);
  const tags = platform?.tags;

  // What the index changed, once it said so (a watch's event) and a moment more.
  const settled = React.useCallback(
    async (before: number) => {
      for (let tries = 0; tries < 100 && heard.current === before; tries += 1) await wait(20);
      await wait(150);
    },
    [heard],
  );

  React.useEffect(() => {
    if (!platform) return;
    say('tags', tags ? 'available' : 'unavailable');
    tags?.names().then(
      (names) => say('names at start', names.map(({ name, count }) => `${name}:${count}`).join(' ') || 'none'),
      (error) => say('names at start', `threw ${error.code}`),
    );
  }, [platform, tags, say]);

  const said = (
    <View style={{ flexDirection: 'row', flexWrap: 'wrap', columnGap: 24 }}>
      {lines.map(([name, value], index) => (
        <Muted key={index} style={{ width: 460, fontSize: 12, lineHeight: 16 }}>
          {`${name}: ${value}`}
        </Muted>
      ))}
    </View>
  );
  if (!platform || !tags || !home) return <View style={{ padding: 16 }}>{said}</View>;
  const documents = new Directory(home, 'Documents');
  const uriIn = (...parts: string[]) => new File(documents, ...parts).uri;
  const relative = (uri: string) => decodeURIComponent(uri.slice(home.length));
  const found = async (name: string) =>
    (await tags.tagged(name))
      .map((item) => `${relative(item.uri)}${item.isDirectory ? '/' : ''}`)
      .sort()
      .join(' ') || 'none';

  const tag = async () => {
    const before = heard.current;
    await attempt(say, 'tagged', async () =>
      outcomes(
        await tags.change([uriIn('report.txt'), uriIn('plan.txt'), 'file:///etc/hostname'], {
          add: [
            { name: 'Work', color: 'red' },
            { name: 'Q3', color: null },
          ],
        }),
      ),
    );
    await attempt(say, 'in Documents', async () => {
      const held = await tags.inFolder(documents.uri);
      return Object.keys(held)
        .sort()
        .map((uri) => `${relative(uri).replace('Documents/', '')}: ${shown(held[uri])}`)
        .join(', ');
    });
    await attempt(say, 'of items', async () =>
      (await tags.ofItems([uriIn('report.txt'), uriIn('gone.txt')])).map(shown).join(' | '),
    );
    await attempt(say, 'bad name', () => tags.change([uriIn('report.txt')], { add: [{ name: 'a,b', color: null }] }));
    await settled(before);
    await attempt(say, 'index has Work', () => found('work'));
  };

  const mac = async () => {
    await attempt(say, 'mac changed', async () =>
      outcomes(
        await tags.change([uriIn('mac.pages')], { add: [{ name: 'Work', color: 'blue' }], remove: ['clients'] }),
      ),
    );
    await attempt(say, 'mac now', async () => shown((await tags.ofItems([uriIn('mac.pages')]))[0]));
    // Remove first, then add: a tag respelt where it was.
    await attempt(say, 'respelt', async () => {
      await tags.change([uriIn('plan.txt')], { remove: ['work'], add: [{ name: 'WORK', color: null }] });
      return shown((await tags.ofItems([uriIn('plan.txt')]))[0]);
    });
  };

  const copyAndMove = async () => {
    const before = heard.current;
    await attempt(say, 'copy', async () => {
      const copy = new File(home, 'Downloads', 'report.txt');
      await new File(documents, 'report.txt').copy(new Directory(home, 'Downloads'));
      return shown((await tags.ofItems([copy.uri]))[0]);
    });
    await attempt(say, 'move', async () => {
      const plan = new File(documents, 'plan.txt');
      await plan.move(new Directory(documents, 'Projects'));
      return `${relative(plan.uri)} ${shown((await tags.ofItems([plan.uri]))[0])}`;
    });
    await settled(before);
    await attempt(say, 'index has Q3', () => found('q3'));
  };

  const renameAndTrash = async () => {
    let before = heard.current;
    const report = new File(documents, 'report.txt');
    await attempt(say, 'renamed', () => {
      report.rename('final.txt');
      return relative(report.uri);
    });
    await settled(before);
    await attempt(say, 'index after rename', () => found('Q3'));
    before = heard.current;
    const trashed = await platform.trash([report.uri]);
    say('trashed', outcomes(trashed));
    await settled(before);
    await attempt(say, 'index after trash', () => found('Q3'));
    await attempt(say, 'in the Trash', async () => {
      const held = (await platform.trashed()) as readonly TrashedWithTags[];
      return held.map((item) => `${item.name}: ${shown(item.tags ?? null)}`).join(', ');
    });
    before = heard.current;
    say('restored', outcomes(await platform.restore(trashed.map((item) => item.result!))));
    await settled(before);
    await attempt(say, 'index after restore', () => found('Q3'));
    await attempt(
      say,
      'names',
      async () => (await tags.names()).map(({ name, count }) => `${name}:${count}`).join(' ') || 'none',
    );
  };

  const anotherProgram = async () => {
    say('tags', 'waiting for another program');
    // Another program tags a file (setfattr): the watch says the index changed.
    for (let tries = 0; tries < 50; tries += 1) {
      const before = heard.current;
      await settled(before);
      const later = await found('later');
      if (later !== 'none') {
        say('another program', later);
        return;
      }
    }
    say('another program', 'not heard');
  };

  return (
    <View style={{ padding: 16, gap: 4 }}>
      <View style={{ flexDirection: 'row', gap: 12, marginBottom: 8 }}>
        <Button onPress={tag}>Tag</Button>
        <Button onPress={mac}>Mac</Button>
        <Button onPress={copyAndMove}>Copy and move</Button>
        <Button onPress={renameAndTrash}>Rename and Trash</Button>
        <Button onPress={anotherProgram}>Another program</Button>
      </View>
      {said}
    </View>
  );
}

export default function FileTagsProbe() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <Probe />
    </BloomProvider>
  );
}
