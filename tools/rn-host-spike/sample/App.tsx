import * as React from 'react';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { BloomScope } from '@oxy.so/bloom/appearance';
import { AppShell } from '@oxy.so/bloom/app-shell';
import { Screen, ScreenScrollView } from '@oxy.so/bloom/screen';
import { PageHeader } from '@oxy.so/bloom/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@oxy.so/bloom/card';
import { Button } from '@oxy.so/bloom/button';
import { TextField, TextFieldInput } from '@oxy.so/bloom/text-field';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '@oxy.so/bloom/dropdown-menu';
import { oxyosSurfaces } from './oxyos/platform-surface';

const ACTIONS = ['Reply', 'Forward', 'Archive', 'Move to folder', 'Mark as unread', 'Delete'];
import { H3, Muted, P } from '@oxy.so/bloom/typography';

const MESSAGES = [
  { id: 'a', title: 'Native Bloom on OxyOS', body: 'Rendered by Fabric and Hermes on Linux.' },
  { id: 'b', title: 'Popups beyond windows', body: 'The compositor places menus outside the app.' },
  { id: 'c', title: 'No X11 or Wayland', body: 'A custom display protocol carries every frame.' },
  { id: 'd', title: 'One React tree', body: 'Panels share providers, theme and state.' },
  { id: 'e', title: 'Native scrolling', body: 'The host owns each scroll offset, as Android does.' },
  { id: 'f', title: 'Touch events', body: 'Pointer presses reach Pressability unchanged.' },
  { id: 'g', title: 'Shared memory', body: 'Frames travel in sealed buffers, read with pread.' },
  { id: 'h', title: 'One frame in flight', body: 'The host waits for feedback before drawing again.' },
  { id: 'i', title: 'Worklets', body: 'Animations run on the UI runtime, committed by Fabric.' },
  { id: 'j', title: 'Last message', body: 'Scrolled into view by the wheel.' },
];

function Inbox() {
  return (
    <Screen header={<PageHeader title="Inbox" />}>
      <ScreenScrollView testID="inbox">
        {MESSAGES.map((message) => (
          <Card key={message.id} style={{ marginHorizontal: 12, marginBottom: 8 }}>
            <CardHeader>
              <CardTitle>{message.title}</CardTitle>
              <CardDescription>{message.body}</CardDescription>
            </CardHeader>
          </Card>
        ))}
      </ScreenScrollView>
    </Screen>
  );
}

function Detail() {
  const [presses, setPresses] = React.useState(0);
  const [reply, setReply] = React.useState('');
  const [choice, setChoice] = React.useState<string | null>(null);
  return (
    <Screen header={<PageHeader title="Native Bloom on OxyOS" />}>
      <ScreenScrollView contentContainerStyle={{ padding: 16, gap: 12 }}>
        <H3>Actual Bloom components</H3>
        <P>
          BloomProvider, BloomScope at size md, AppShell with a two-panel layout, Screens with
          PageHeaders, Cards, typography and a Button, drawn by the OxyOS host into a buffer.
        </P>
        <Muted>React Native 0.88 · Hermes · Fabric · Yoga</Muted>
        <Button testID="press-me" onPress={() => setPresses((count) => count + 1)}>
          {presses === 0 ? 'Press me' : `Pressed ${presses} ${presses === 1 ? 'time' : 'times'}`}
        </Button>
        <TextField>
          <TextFieldInput
            testID="reply"
            placeholder="Write a reply"
            value={reply}
            onChangeText={setReply}
          />
        </TextField>
        <Muted>{reply ? `Reply: ${reply}` : 'No reply yet'}</Muted>
        {/* Opens to the right of its trigger, past the window's edge: a compositor popup. */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild style={{ alignSelf: 'flex-end' }} testID="open-menu">
            <Button appearance="outline">Open menu</Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent side="right" align="start" label="Message actions" testID="message-actions">
            {ACTIONS.map((action) => (
              <DropdownMenuItem key={action} testID={`action-${action}`} onPress={() => setChoice(action)}>
                {action}
              </DropdownMenuItem>
            ))}
            {/* An inline submenu: expanding it grows the menu, which the compositor re-places. */}
            <DropdownMenuSub>
              <DropdownMenuSubTrigger testID="more-actions">More actions</DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                {['Copy link', 'Print'].map((action) => (
                  <DropdownMenuItem key={action} testID={`action-${action}`} onPress={() => setChoice(action)}>
                    {action}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          </DropdownMenuContent>
        </DropdownMenu>
        <Muted>{choice ? `Chosen: ${choice}` : 'Nothing chosen'}</Muted>
      </ScreenScrollView>
    </Screen>
  );
}

export default function App() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <BloomScope size="md">
        <AppShell
          panelLayout={{
            mode: 'split',
            gap: 8,
            panels: [
              { id: 'inbox', width: 340, content: <Inbox /> },
              { id: 'detail', content: <Detail /> },
            ],
          }}
        />
      </BloomScope>
    </BloomProvider>
  );
}
