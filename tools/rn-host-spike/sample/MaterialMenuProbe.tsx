/**
 * The platform's material (docs/adr/0069-the-platforms-material.mdx), for scenarios/material-*.json:
 * a window of four colour bands, red, green, blue and yellow (each 100 logical pixels wide below a
 * white strip that holds the buttons), with a menu (Bloom's DropdownMenu, in a compositor popup of
 * its own) and a dialog (Bloom's Dialog, drawn in the window over its content) opened over them.
 *
 * Both are Bloom's floating surfaces: with the adapter's `material`, each is the platform's material
 * with its tint over it. The menu's popup has nothing of its own beneath its material, so what shows
 * through it is the compositor's part: the bands beneath the popup, blurred and saturated. The
 * dialog lies over the window's own bands, so what shows through it is the host's part. With Reduce
 * transparency on, Bloom mounts no material: each is its tint, opaque, and the bands never show.
 */
import * as React from 'react';
import { View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { Dialog } from '@oxy.so/bloom/dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@oxy.so/bloom/dropdown-menu';
import { oxyosSurfaces } from './oxyos/platform-surface';

const BANDS = ['#ff0000', '#00c000', '#0000ff', '#ffd000'];

export default function MaterialMenuProbe() {
  const [menuOpen, setMenuOpen] = React.useState(false);
  const [dialogOpen, setDialogOpen] = React.useState(false);
  return (
    <BloomProvider platform={oxyosSurfaces} mode="light">
      <View testID="material-page" style={{ flex: 1, backgroundColor: '#ffffff' }}>
        <View style={{ height: 48, flexDirection: 'row', alignItems: 'center', gap: 12, paddingHorizontal: 12 }}>
          <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
            <DropdownMenuTrigger asChild testID="menu-trigger">
              <Button appearance="outline">Bands</Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent side="bottom" align="start" label="Bands" testID="bands-menu">
              {['First', 'Second', 'Third', 'Fourth', 'Fifth', 'Sixth'].map((item) => (
                <DropdownMenuItem key={item} testID={`band-${item}`} onPress={() => setMenuOpen(false)}>
                  {item}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          <Button appearance="outline" testID="dialog-trigger" onPress={() => setDialogOpen(true)}>
            Dialog
          </Button>
        </View>
        <View testID="bands" style={{ flex: 1, flexDirection: 'row' }}>
          {BANDS.map((colour) => (
            <View key={colour} style={{ width: 100, backgroundColor: colour }} />
          ))}
        </View>
        <Dialog
          open={dialogOpen}
          onClose={() => setDialogOpen(false)}
          title="Material"
          description="Over the window's bands."
          actions={[{ label: 'Done', color: 'default', testID: 'dialog-done' }]}
        />
      </View>
    </BloomProvider>
  );
}
