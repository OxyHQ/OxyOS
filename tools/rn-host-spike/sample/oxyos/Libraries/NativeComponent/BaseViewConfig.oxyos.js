/**
 * React Native's base view config on OxyOS: its Android one, with the W3C pointer presses a view
 * listens to (`onPointerDown`, `onPointerUp` and their capture phases) among the props sent to the
 * host, as its iOS one has them. Android's config lists their bubbling events but not the props,
 * so Fabric's pointer events processor never sees a view listening and no press reaches
 * JavaScript. On OxyOS every mouse button's press and release is a pointer event (input.h,
 * docs/react-native-android-adaptation.mdx): a secondary press opens a context menu, and a click's
 * modifiers (Shift, Ctrl) select as a desktop's list does.
 *
 * And `oxyosTextMenu`, a bubbling event: a text input asking for its context menu
 * (docs/adr/0037-text-editing.mdx), which the runtime's root hears from any input (text-menu.js).
 *
 * And `acceptsFirstMouse`, react-native-macos's View prop (NSView's `acceptsFirstMouse:`): a view
 * that takes the press activating its window, as a Mac's title bar, its window buttons and its
 * text and table views do; any other press that activates a window only activates it
 * (docs/adr/0016-windows.mdx, decision 9). Bloom's window drag regions and controls set it.
 *
 * And `textContentType`, a text input's iOS prop, which React Native sends only to iOS: what the
 * field holds for AutoFill (`username`, `password`, `newPassword`, `oneTimeCode`, `emailAddress`),
 * read by the host beside `autoComplete`, which Android's config sends
 * (docs/adr/0050-keychain-autofill-and-passkeys.mdx). An application sets either, as on a phone.
 */
import Android from 'react-native/Libraries/NativeComponent/BaseViewConfig.android';

const BaseViewConfigOxyOS = {
  ...Android,
  bubblingEventTypes: {
    ...Android.bubblingEventTypes,
    topOxyosTextMenu: {
      phasedRegistrationNames: { bubbled: 'onOxyOSTextMenu', captured: 'onOxyOSTextMenuCapture' },
    },
  },
  validAttributes: {
    ...Android.validAttributes,
    onPointerDown: true,
    onPointerDownCapture: true,
    onPointerUp: true,
    onPointerUpCapture: true,
    acceptsFirstMouse: true,
    textContentType: true,
  },
};

export default BaseViewConfigOxyOS;
