/**
 * Finder's drops on a folder (docs/adr/0047-finder-drags-and-the-desktop.mdx, decisions 8 and 10):
 * one drop target filling the window (the OxyOS adapter's DropTarget, Bloom's DropZone) that takes
 * files into the home, its `folder`, with a copy or a move, a copy its own first choice. A drag of
 * files on the home's volume is answered with a move, as Finder moves on the same volume; Option
 * still copies. It shows the action a drop would take while a drag is over it, each drop with its
 * action, and the window's drag (the adapter's drag.watchWindow, Bloom's useWindowDrag) since it
 * last entered: its entering, and its exit, dropped or not, with how many drops it had by then. It
 * takes nothing itself: what a drop would do to the files is the application's.
 *
 * It learns its folder a while after a drag comes over it (FOLDER_LATE_MS), as a file manager
 * learns which folder is under the pointer: until then it has none and its own copy stands; then
 * the host answers the compositor again as the change commits, the pointer still.
 */
import * as React from 'react';
import { Text, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';

const surfaces = oxyosSurfaces as any;
const FOLDER_LATE_MS = 2000;

function Probe() {
  const [home, setHome] = React.useState<string | null>(null);
  const [over, setOver] = React.useState<string | null>(null);
  const [drops, setDrops] = React.useState<string[]>([]);
  const [windowDrag, setWindowDrag] = React.useState<string[]>([]);
  const [folderKnown, setFolderKnown] = React.useState(false);
  const learning = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const dropped = React.useRef(0);
  const forget = () => {
    if (learning.current) clearTimeout(learning.current);
    learning.current = null;
    setFolderKnown(false);
  };
  React.useEffect(() => {
    surfaces.files?.places().then((places: any[]) => setHome(places.find((place) => place.kind === 'home')?.uri ?? null));
  }, []);
  React.useEffect(
    () =>
      surfaces.drag?.watchWindow?.((event: any) => {
        if (event.type === 'enter') {
          setWindowDrag(['enter']);
          return;
        }
        // How many drops its onDrop had taken when the exit came: told after the drop's.
        const exit = event.dropped ? `exit dropped after ${dropped.current} drops` : 'exit';
        setWindowDrag((before) => [...before, exit]);
      }),
    [],
  );
  const DropTarget = surfaces.DropTarget;
  const body = (
    <View
      style={{
        flex: 1,
        backgroundColor: over ? '#d3f9d8' : '#e7f5ff',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 6,
      }}
    >
      <Text>{over ? `Over: ${over}` : 'Drop in the home'}</Text>
      <Text>{folderKnown ? 'Folder: the home' : 'Folder: not yet'}</Text>
      <Text>{`Drops: ${drops.length}`}</Text>
      {drops.map((drop, index) => (
        <Text key={index}>{drop}</Text>
      ))}
      <Text>{`Window: ${windowDrag.join(', ') || 'no drag yet'}`}</Text>
    </View>
  );
  if (!DropTarget || !home) return body;
  return (
    <DropTarget
      testID="folder-zone"
      accept={['files']}
      actions={['copy', 'move']}
      folder={folderKnown ? home : undefined}
      style={{ flex: 1 }}
      onDragEnter={({ action }: any) => {
        setOver(action ?? 'nothing');
        forget();
        learning.current = setTimeout(() => setFolderKnown(true), FOLDER_LATE_MS);
      }}
      onDragOver={({ action }: any) => setOver(action ?? 'nothing')}
      onDragLeave={() => {
        setOver(null);
        forget();
      }}
      onDrop={({ action, data }: any) => {
        dropped.current += 1;
        const files = data.files.map((file: any) => file.name).join(', ');
        setDrops((before) => [...before, `${files} (${action})`]);
      }}
    >
      {body}
    </DropTarget>
  );
}

export default function FolderDropProbe() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <Probe />
    </BloomProvider>
  );
}
