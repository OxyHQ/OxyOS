// The window's key presses offered to JavaScript before the focused view (Bloom's
// `PlatformKeyboard`, docs/adr/0021-menu-bar.mdx): a menu's shortcut runs its item wherever focus
// is, as a Mac's key equivalents do. Each press of a key that is not a modifier becomes a W3C
// `KeyboardEvent` ({ key, code, altKey, ctrlKey, metaKey, shiftKey, repeat }); a press JavaScript
// takes is not delivered to the view, nor is its release.
//
// The compositor sends Linux input codes; the key a code names is read on the US layout here (the
// compositor's keymap is not the host's yet), so a chord is matched by its key's place on a US
// keyboard on any layout.
#pragma once

#include <oxyos_client.h>

#include <folly/dynamic.h>

#include <optional>
#include <set>

namespace oxyos {

class KeyChords {
 public:
  // A key event as received: the press to offer, if it is one; tracks the modifiers held.
  std::optional<folly::dynamic> press(const OxyOSEvent& event);
  // JavaScript took the press of `code`: its release is not delivered either.
  void taken(uint32_t code) { taken_.insert(code); }
  // Whether this event is the release of a key JavaScript took (and so is not delivered).
  bool swallow(const OxyOSEvent& event);

 private:
  bool ctrl_ = false, shift_ = false, alt_ = false, meta_ = false;
  std::set<uint32_t> held_, taken_;
};

}  // namespace oxyos
