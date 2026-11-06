/**
 * What the dock's drags need of the platform (docs/adr/0035-drag-and-drop.mdx and
 * docs/adr/0034-the-dock.mdx, 6 November 2026), a shell's side, run as the session's shell: a
 * panel at the output's bottom with three drop targets, as a dock's icons are.
 * - "Abrir con Inbox": takes files only of a type Inbox opens (acceptFileTypes text/markdown), and
 *   opens what is dropped with Inbox through the session (OpenFiles, citing the drop);
 * - "Tipos": takes any files and shows the types the drag says of them;
 * - "Mantener": takes an application dragged (`application/x-oxyos-application`), as a dock keeps one.
 */
import * as React from 'react';
import { Text, View } from 'react-native';
import { oxyosShell } from './oxyos/shell-surface';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { useOxyOSShell } from './oxyos/shell-state';

const surfaces = oxyosSurfaces as any;
const APPLICATION = 'application/x-oxyos-application';

function Zone({
  label,
  shown,
  ...target
}: {
  label: string;
  shown: string;
  accept: string[];
  actions: string[];
  acceptFileTypes?: string[];
  onDragEnter?: (event: any) => void;
  onDragOver?: (event: any) => void;
  onDrop: (event: any) => void;
}) {
  const DropTarget = surfaces.DropTarget;
  const body = (
    <View
      accessible
      accessibilityLabel={label}
      style={{ width: 240, height: 80, borderRadius: 12, backgroundColor: '#e7f5ff', justifyContent: 'center', alignItems: 'center' }}
    >
      <Text>{label}</Text>
      <Text>{shown}</Text>
    </View>
  );
  return DropTarget ? (
    <DropTarget testID={label} {...target}>
      {body}
    </DropTarget>
  ) : (
    body
  );
}

export default function ShellDropProbe() {
  const shell = useOxyOSShell() as any;
  const [opened, setOpened] = React.useState('nothing');
  const [seen, setSeen] = React.useState('nothing');
  const [kept, setKept] = React.useState('nothing');
  const Surface = oxyosShell.Surface as any;
  // At the output's bottom, where the shell's layout puts a dock: the host draws the region where
  // the layout has it.
  return (
    <View style={{ flex: 1, justifyContent: 'flex-end' }}>
    <Surface kind="panel" edge="bottom" exclusive style={{ height: 100 }}>
      <View style={{ flex: 1, flexDirection: 'row', gap: 20, padding: 10, backgroundColor: '#f8f9fa', justifyContent: 'center' }}>
        <Zone
          label="Abrir con Inbox"
          shown={`Abierto: ${opened}`}
          accept={['files']}
          actions={['copy']}
          acceptFileTypes={['text/markdown']}
          onDrop={({ data }) => {
            const uris = data.files.map((file: any) => file.uri);
            const sent = shell.openFiles('so.oxy.inbox', uris);
            setOpened(`${data.files.map((file: any) => file.name).join(', ')} (${sent ? 'sent' : 'not sent'})`);
          }}
        />
        <Zone
          label="Tipos"
          shown={`Visto: ${seen}`}
          accept={['files']}
          actions={['copy']}
          onDragEnter={({ fileTypes }) => setSeen((fileTypes ?? []).join(', ') || 'none')}
          onDrop={() => {}}
        />
        <Zone
          label="Mantener"
          shown={`Mantenido: ${kept}`}
          accept={[APPLICATION]}
          actions={['move']}
          onDrop={({ data }) => setKept(data.other[APPLICATION] ?? 'nothing')}
        />
      </View>
    </Surface>
    </View>
  );
}
