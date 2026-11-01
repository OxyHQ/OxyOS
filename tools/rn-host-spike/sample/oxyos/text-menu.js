/**
 * A text field's context menu, the platform's for every React Native TextInput on OxyOS
 * (docs/adr/0037-text-editing.mdx), as a Mac's text fields have Cut, Copy and Paste from AppKit,
 * Android's EditText its selection actions and a browser's input its own menu: the application
 * draws none of it.
 *
 * The host opens it (textinput.h, `TextEditing::contextMenu`): a secondary click or a
 * Control-click on an input focuses it, selects the word under the pointer unless the click is
 * within the selection, and sends the input's bubbling `oxyosTextMenu` event with the point in the
 * input and which commands apply now. The runtime wraps every application's root in
 * `TextFieldMenus` (AppRegistry's wrapper, framework-init.js), which hears that event from any
 * input and shows Bloom's ContextMenu in the platform's popup, hanging from the input at the point
 * (Bloom's `anchorRef`), as the dock shows its menu (ADR 0034): announced as a menu, driven by
 * Bloom's popup keys (arrows, Home, End, Return, Escape), the rows that do not apply disabled. A row
 * chosen is a command to the input (`oxyosEdit`), which the host performs as the input's own key
 * would, citing the row's press or key for the clipboard. The input keeps its focus and selection
 * throughout; the keyboard comes back to it as the popup closes.
 */
import * as React from 'react';
import { StyleSheet, View } from 'react-native';
import { dispatchCommand } from 'react-native/Libraries/ReactNative/RendererProxy';
import { BloomProvider } from '@oxy.so/bloom/provider';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
} from '@oxy.so/bloom/context-menu';
import { oxyosSurfaces } from './platform-surface';

// The commands' names in the session's language (Spanish or English, as Settings and the shell
// speak), as a Mac's Edit menu names them. The chords as the session shows them: Ctrl is the
// applications' ⌘ on OxyOS (ADR 0024).
const WORDS = {
  en: { undo: 'Undo', redo: 'Redo', cut: 'Cut', copy: 'Copy', paste: 'Paste', selectAll: 'Select All' },
  es: { undo: 'Deshacer', redo: 'Rehacer', cut: 'Cortar', copy: 'Copiar', paste: 'Pegar', selectAll: 'Seleccionar todo' },
};
const CHORDS = {
  undo: 'Ctrl+Z',
  redo: 'Ctrl+Shift+Z',
  cut: 'Ctrl+X',
  copy: 'Ctrl+C',
  paste: 'Ctrl+V',
  selectAll: 'Ctrl+A',
};
// Rows in a Mac's order, Undo and Redo first as Windows and GTK add them, groups between lines.
const GROUPS = [['undo', 'redo'], ['cut', 'copy', 'paste'], ['selectAll']];

function words() {
  let locale = '';
  try {
    locale = new Intl.DateTimeFormat().resolvedOptions().locale;
  } catch {
    // Without Intl, English.
  }
  return locale.toLowerCase().split(/[-_]/)[0] === 'es' ? WORDS.es : WORDS.en;
}

/** The menu of the input it was opened for, while open. */
function TextFieldMenu({ menu, onClose }) {
  const said = React.useMemo(words, []);
  const choose = React.useCallback(
    (command) => () => {
      // The input's own instance (the event's target), which the host knows by its tag.
      dispatchCommand(menu.field.current, 'oxyosEdit', [command]);
    },
    [menu],
  );
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <ContextMenu anchor={menu.anchor} anchorRef={menu.field} onAnchorChange={(anchor) => anchor === null && onClose()}>
        <ContextMenuContent testID="text-field-menu">
          {GROUPS.map((group, index) => (
            <React.Fragment key={group[0]}>
              {index > 0 && <ContextMenuSeparator />}
              {group.map((command) => (
                <ContextMenuItem
                  key={command}
                  disabled={!menu.commands[command]}
                  onPress={choose(command)}
                  trailing={<ContextMenuShortcut>{CHORDS[command]}</ContextMenuShortcut>}
                  accessibilityLabel={said[command]}
                >
                  {said[command]}
                </ContextMenuItem>
              ))}
            </React.Fragment>
          ))}
        </ContextMenuContent>
      </ContextMenu>
    </BloomProvider>
  );
}

/**
 * Every application's root, as AppRegistry's wrapper: the application, and the menu of the input
 * that last asked for one. One view filling the window, as React Native's own container does, so
 * the application's layout is unchanged.
 */
export function TextFieldMenus({ children }) {
  const [menu, setMenu] = React.useState(null);
  const open = React.useCallback((event) => {
    const field = event.target;
    const { x, y, undo, redo, cut, copy, paste, selectAll } = event.nativeEvent;
    if (!field || typeof field.measureInWindow !== 'function') return;
    // Bloom's anchor is a point in the window: the input's corner there plus the point in it.
    field.measureInWindow((left, top) => {
      setMenu({
        field: { current: field },
        anchor: { x: left + x, y: top + y },
        commands: { undo, redo, cut, copy, paste, selectAll },
      });
    });
  }, []);
  const close = React.useCallback(() => setMenu(null), []);
  return (
    <View style={styles.root} collapsable={false} pointerEvents="box-none" onOxyOSTextMenu={open}>
      {children}
      {menu && <TextFieldMenu key={menu.anchor.x + ',' + menu.anchor.y} menu={menu} onClose={close} />}
    </View>
  );
}

const styles = StyleSheet.create({ root: { flex: 1 } });
