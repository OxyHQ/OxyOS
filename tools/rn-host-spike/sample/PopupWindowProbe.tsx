/**
 * Popups and their window, for scenarios/popup-window*.json: a window as ADR 0016 draws it (Bloom's
 * settings layout, the controls at the top of its rail, which moves the window) whose page opens
 * each of Bloom's popups in a compositor popup of its own: a menu (DropdownMenu), a popover, a
 * context menu and a tooltip. The page says what the application heard: the window's state, and
 * how many times each popup opened and closed, so a scenario sees a popup closed once and never
 * opened again by itself.
 */
import * as React from 'react';
import { View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from '@oxy.so/bloom/context-menu';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@oxy.so/bloom/dropdown-menu';
import { RiInformationLine, RiPaletteLine } from '@oxy.so/bloom/icons';
import { Popover, PopoverContent, PopoverTrigger } from '@oxy.so/bloom/popover';
import { SettingsCard, SettingsLayout, SettingsRow } from '@oxy.so/bloom/settings-modal';
import type { SettingsModalPage, SettingsNavGroup } from '@oxy.so/bloom/settings-modal';
import { Tooltip, TooltipTextBubble, TooltipTrigger } from '@oxy.so/bloom/tooltip';
import { Muted, Text } from '@oxy.so/bloom/typography';
import { useWindow } from '@oxy.so/bloom/window';
import { oxyosSurfaces } from './oxyos/platform-surface';

const GROUPS: SettingsNavGroup[] = [
  {
    label: 'Settings',
    items: [
      { key: 'popups', label: 'Popups', icon: RiPaletteLine, page: 'popups' },
      { key: 'about', label: 'About', icon: RiInformationLine, page: 'about' },
    ],
  },
];

type Counts = { opened: number; closed: number };
const NONE: Counts = { opened: 0, closed: 0 };

/** Counts an `onOpenChange`'s openings and closings. */
function useCounted(): [boolean, (open: boolean) => void, Counts] {
  const [open, setOpen] = React.useState(false);
  const [counts, setCounts] = React.useState(NONE);
  const change = React.useCallback((next: boolean) => {
    setOpen(next);
    setCounts((was) => (next ? { ...was, opened: was.opened + 1 } : { ...was, closed: was.closed + 1 }));
  }, []);
  return [open, change, counts];
}

const said = (name: string, { opened, closed }: Counts) => `${name} opened ${opened}, closed ${closed}`;

function PopupsPage() {
  const state = useWindow();
  const yes = (on: boolean | undefined) => (on ? 'yes' : 'no');
  const [menuOpen, setMenuOpen, menu] = useCounted();
  const [popoverOpen, setPopoverOpen, popover] = useCounted();
  const [, setContextOpen, context] = useCounted();
  const [tip, setTip, tooltip] = useCounted();
  const [chosen, setChosen] = React.useState('nothing');
  return (
    <SettingsCard>
      <SettingsRow label="Window">
        <Text testID="window-state">
          {state
            ? `active ${yes(state.active)}, zoomed ${yes(state.maximized)}, full screen ${yes(state.fullscreen)}`
            : 'no window'}
        </Text>
      </SettingsRow>
      <View style={{ padding: 16, gap: 12, alignItems: 'flex-start' }}>
        <View style={{ flexDirection: 'row', gap: 12 }}>
          <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
            <DropdownMenuTrigger asChild testID="menu-trigger">
              <Button appearance="outline">Actions</Button>
            </DropdownMenuTrigger>
            {/* As wide as it was before Bloom's desktop menus took their content's width: 266
              logical pixels are no whole number of device pixels at 1.25, so a move onto such a
              display still has the menu laid out again at 265 (popup-window-displays). */}
            <DropdownMenuContent side="bottom" align="start" label="Actions" testID="actions" minWidth={266}>
              {['Archive', 'Delete'].map((action) => (
                <DropdownMenuItem key={action} testID={`action-${action}`} onPress={() => setChosen(action)}>
                  {action}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          <Popover open={popoverOpen} onOpenChange={setPopoverOpen}>
            <PopoverTrigger asChild>
              <Button appearance="outline" testID="popover-trigger">
                Details
              </Button>
            </PopoverTrigger>
            <PopoverContent label="Details" testID="details">
              <Muted>Shown beside its button</Muted>
              <Button testID="details-done" onPress={() => setPopoverOpen(false)}>
                Done
              </Button>
            </PopoverContent>
          </Popover>
          <Tooltip visible={tip} onVisibleChange={setTip} position="bottom" delay={0}>
            <TooltipTrigger>
              <Button appearance="outline" testID="tip-trigger" onPress={() => setTip(!tip)}>
                Help
              </Button>
            </TooltipTrigger>
            <TooltipTextBubble>Explains the page</TooltipTextBubble>
          </Tooltip>
        </View>
        <ContextMenu onOpenChange={setContextOpen}>
          <ContextMenuTrigger label="Area" testID="context-area">
            <View style={{ width: 280, height: 80, backgroundColor: '#dfe7f5', borderRadius: 12 }} />
          </ContextMenuTrigger>
          <ContextMenuContent>
            <ContextMenuItem onPress={() => setChosen('Rename')}>Rename</ContextMenuItem>
            <ContextMenuItem onPress={() => setChosen('Duplicate')}>Duplicate</ContextMenuItem>
          </ContextMenuContent>
        </ContextMenu>
        <Muted testID="counts-menus">{`${said('menu', menu)}; ${said('context', context)}`}</Muted>
        <Muted testID="counts-others">{`${said('popover', popover)}; ${said('tooltip', tooltip)}`}</Muted>
        <Muted testID="chosen">{`Chosen: ${chosen}`}</Muted>
      </View>
    </SettingsCard>
  );
}

const PAGES: Record<string, SettingsModalPage> = {
  popups: { title: 'Popups', content: <PopupsPage /> },
  about: {
    title: 'About',
    content: (
      <SettingsCard>
        <SettingsRow label="OxyOS" />
      </SettingsCard>
    ),
  },
};

export default function PopupWindowProbe() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <SettingsLayout groups={GROUPS} pages={PAGES} defaultPage="popups" testID="settings" />
    </BloomProvider>
  );
}
