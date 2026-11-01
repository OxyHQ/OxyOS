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
  },
};

export default BaseViewConfigOxyOS;
