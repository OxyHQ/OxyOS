/**
 * An application window as ADR 0016 draws it, for scenarios/windows.json: Bloom's settings layout
 * in the window, the window's controls (Bloom's) at the top of its rail, which keeps their room
 * and moves the window. The page says the window's state as the application receives it.
 */
import * as React from 'react';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { RiInformationLine, RiPaletteLine } from '@oxy.so/bloom/icons';
import { SettingsCard, SettingsLayout, SettingsRow } from '@oxy.so/bloom/settings-modal';
import type { SettingsModalPage, SettingsNavGroup } from '@oxy.so/bloom/settings-modal';
import { Text } from '@oxy.so/bloom/typography';
import { useWindow } from '@oxy.so/bloom/window';
import { oxyosSurfaces } from './oxyos/platform-surface';

const GROUPS: SettingsNavGroup[] = [
  {
    label: 'Settings',
    items: [
      { key: 'appearance', label: 'Appearance', icon: RiPaletteLine, page: 'appearance' },
      { key: 'about', label: 'About', icon: RiInformationLine, page: 'about' },
    ],
  },
];

function WindowState() {
  const state = useWindow();
  const said = (on: boolean | undefined) => (on ? 'yes' : 'no');
  return (
    <SettingsCard>
      <SettingsRow label="Window">
        <Text testID="window-state">
          {state
            ? `active ${said(state.active)}, zoomed ${said(state.maximized)}, full screen ${said(state.fullscreen)}`
            : 'no window'}
        </Text>
      </SettingsRow>
    </SettingsCard>
  );
}

const PAGES: Record<string, SettingsModalPage> = {
  appearance: { title: 'Appearance', content: <WindowState /> },
  about: {
    title: 'About',
    content: (
      <SettingsCard>
        <SettingsRow label="OxyOS" />
      </SettingsCard>
    ),
  },
};

export default function WindowProbe() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <SettingsLayout groups={GROUPS} pages={PAGES} defaultPage="appearance" testID="settings" />
    </BloomProvider>
  );
}
