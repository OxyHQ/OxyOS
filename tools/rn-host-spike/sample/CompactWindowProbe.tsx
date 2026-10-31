/** Native composition contract: compact navigation leaves the window controls' room to its header. */
import * as React from 'react';
import { View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { AppShell } from '@oxy.so/bloom/app-shell';
import { Button } from '@oxy.so/bloom/button';
import { RiHomeLine } from '@oxy.so/bloom/icons/RiHomeLine';
import { PageHeader } from '@oxy.so/bloom/page-header';
import { Screen, ScreenScrollView } from '@oxy.so/bloom/screen';
import { Text } from '@oxy.so/bloom/typography';
import { useWindow } from '@oxy.so/bloom/window';
import { oxyosSurfaces } from './oxyos/platform-surface';

function Workspace({ adaptive }: { adaptive: boolean }) {
  const [collapsed, setCollapsed] = React.useState(true);
  const [measurement, setMeasurement] = React.useState('Header not measured');
  const [widthMeasurement, setWidthMeasurement] = React.useState('Panel not measured');
  const [resizeMeasurement, setResizeMeasurement] = React.useState('Panel resize not measured');
  const phase = React.useRef(0);
  const expandedWidth = React.useRef<number | null>(null);
  const collapsedWidth = React.useRef<number | null>(null);
  const changeCollapsed = (value: boolean) => {
    phase.current += 1;
    if (!value) expandedWidth.current = null;
    setCollapsed(value);
    setMeasurement('Header not measured');
    setWidthMeasurement('Panel not measured');
    collapsedWidth.current = null;
    setResizeMeasurement('Panel resize not measured');
  };
  const title = React.useRef<React.ComponentRef<typeof View>>(null);
  const bodyRef = React.useRef<React.ComponentRef<typeof View>>(null);
  const measure = () => {
    const measuredPhase = phase.current;
    title.current?.measureInWindow((x) => {
      if (phase.current !== measuredPhase) return;
      setMeasurement(`Header room ${x >= 88 ? 'yes' : 'no'} (${Math.round(x)})`);
    });
    bodyRef.current?.measureInWindow((_x, _y, width) => {
      if (phase.current !== measuredPhase) return;
      if (!collapsed) {
        expandedWidth.current = width;
        setWidthMeasurement(`Panel width recorded (${Math.round(width)})`);
      } else if (expandedWidth.current === null) {
        setWidthMeasurement(`Panel width baseline (${Math.round(width)})`);
      } else {
        // The sm docked sidebar changes from 232 to 45 points: 187 reclaimed.
        // Allow 27 points for chrome/rounding while rejecting a stale body width.
        const recovered = width - expandedWidth.current;
        setWidthMeasurement(
          `Panel width recovered ${recovered >= 160 ? 'yes' : 'no'} (${Math.round(recovered)})`,
        );
        const previousWidth = collapsedWidth.current;
        collapsedWidth.current = width;
        setResizeMeasurement(
          previousWidth === null
            ? 'Panel resize baseline'
            : `Panel resize ${width - previousWidth >= 80 ? 'yes' : 'no'} (${Math.round(width - previousWidth)})`,
        );
      }
    });
  };
  const window = useWindow();
  const header = (
    <PageHeader
      testID="composition-header"
      title={
        <View testID="composition-title" ref={title} collapsable={false}>
          <Text>Header target</Text>
        </View>
      }
    />
  );
  const sidebar = {
    testID: 'composition-sidebar',
    size: 'sm' as const,
    surface: 'docked' as const,
    collapsed,
    onCollapsedChange: changeCollapsed,
    showSearch: false,
    showThemeToggle: false,
    items: [{ key: 'home', label: 'Home', icon: RiHomeLine, onPress() {} }],
    selected: 'home',
  };
  const body = (
    <View
      ref={bodyRef}
      testID="composition-body"
      collapsable={false}
      style={{ padding: 24, gap: 16 }}
    >
      <Button onPress={() => changeCollapsed(!collapsed)}>Toggle sidebar</Button>
      <Button onPress={measure}>Measure header</Button>
      <Text>{collapsed ? 'Sidebar collapsed' : 'Sidebar expanded'}</Text>
      <Text>{measurement}</Text>
      <Text>{widthMeasurement}</Text>
      <Text>{resizeMeasurement}</Text>
      <Text>{window ? 'Native window connected' : 'No native window'}</Text>
    </View>
  );
  return adaptive ? (
    <AppShell
      testID="composition-shell"
      navigationPlacement="sidebar"
      sidebar={sidebar}
      scroll="auto"
      header={header}
    >
      {body}
    </AppShell>
  ) : (
    <AppShell
      testID="composition-shell"
      contentMaxWidth="none"
      navigationPlacement="sidebar"
      sidebar={sidebar}
      header={null}
      panelLayout={{
        mode: 'split',
        flow: 'bounded',
        panels: [
          {
            id: 'main',
            content: (
              <Screen testID="composition-screen" scroll="auto" header={header}>
                <ScreenScrollView>{body}</ScreenScrollView>
              </Screen>
            ),
          },
        ],
      }}
    />
  );
}

export function CompactPanelWindowProbe() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <Workspace adaptive={false} />
    </BloomProvider>
  );
}
export function CompactAdaptiveWindowProbe() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <Workspace adaptive />
    </BloomProvider>
  );
}
