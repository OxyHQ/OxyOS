import * as React from 'react';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { BloomScope } from '@oxy.so/bloom/appearance';
import { AppShell } from '@oxy.so/bloom/app-shell';
import { Screen, ScreenScrollView } from '@oxy.so/bloom/screen';
import { PageHeader } from '@oxy.so/bloom/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@oxy.so/bloom/card';
import { Button } from '@oxy.so/bloom/button';
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
      <ScreenScrollView>
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
  return (
    <Screen header={<PageHeader title="Native Bloom on OxyOS" />}>
      <ScreenScrollView contentContainerStyle={{ padding: 16, gap: 12 }}>
        <H3>Actual Bloom components</H3>
        <P>
          BloomProvider, BloomScope at size md, AppShell with a two-panel layout, Screens with
          PageHeaders, Cards, typography and a Button, drawn by the OxyOS host into a buffer.
        </P>
        <Muted>React Native 0.88 · Hermes · Fabric · Yoga</Muted>
        <Button onPress={() => setPresses((count) => count + 1)}>
          {presses === 0 ? 'Press me' : `Pressed ${presses} ${presses === 1 ? 'time' : 'times'}`}
        </Button>
      </ScreenScrollView>
    </Screen>
  );
}

export default function App() {
  return (
    <BloomProvider>
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
