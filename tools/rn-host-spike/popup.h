// Popups for Bloom: the `OxyOSPopupView` host component (sample/oxyos/platform-surface.js renders it
// as the OxyOS adapter's PopupSurface) and the compositor popup each mounted one becomes.
//
// The popup view stays in the app's React tree, like React Native's Modal: providers and state
// reach its content as they reach any child. The host leaves its subtree out of the window and
// draws it in a protocol popup of its own, placed by the compositor from the anchor's frame and
// Bloom's placement. The compositor's dismissal is sent back as the view's `onDismiss`.
#pragma once

#include "accessibility.h"
#include "input.h"
#include "renderer.h"
#include "window.h"

#include <react/renderer/components/view/ConcreteViewShadowNode.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

#include <functional>
#include <map>
#include <optional>
#include <set>
#include <string>

namespace facebook::react {

extern const char OxyOSPopupViewComponentName[];

class OxyOSPopupViewProps final : public ViewProps {
 public:
  OxyOSPopupViewProps() = default;
  OxyOSPopupViewProps(const PropsParserContext& context, const OxyOSPopupViewProps& sourceProps, const RawProps& rawProps);

  // The anchor view's tag, and Bloom's placement against it.
  Tag anchorTag{-1};
  std::string side{"bottom"};
  std::string align{"start"};
  Float sideOffset{0};
  Float alignOffset{0};
  // A point in the anchor view (from its top-left corner) to place against instead of its frame:
  // a context menu opens where the pointer pressed (Bloom's `PopupSurfaceProps.point`).
  bool hasPoint{false};
  Float pointX{0};
  Float pointY{0};
};

using OxyOSPopupViewShadowNode = ConcreteViewShadowNode<OxyOSPopupViewComponentName, OxyOSPopupViewProps>;
using OxyOSPopupViewComponentDescriptor = ConcreteComponentDescriptor<OxyOSPopupViewShadowNode>;

}  // namespace facebook::react

namespace oxyos {

// Bloom's placement as the protocol's positioner (docs/adr/0002): flip on the side axis, slide on
// both. `anchor` is the anchor's window-local frame.
OxyOSPositioner positioner(const facebook::react::OxyOSPopupViewProps& props, facebook::react::Rect anchor,
                         facebook::react::Size size);

class PopupHost {
 public:
  // The surface a view is drawn in, and where that surface's origin is in root coordinates: a
  // popup's parent is the surface its anchor is in (a shell panel's, for the shell's menus).
  using Parent = std::function<std::pair<uint32_t, facebook::react::Point>(facebook::react::Tag)>;

  PopupHost(const ViewTree& tree, Window& window, AccessibilityBridge& accessibility, Parent parent = {})
      : tree_(tree), window_(window), accessibility_(accessibility), parent_(std::move(parent)) {}

  // Opens a popup for each newly mounted popup view (grabbed with `serial`, the input that opened
  // it) and destroys the popups whose view went away.
  void sync(uint64_t serial);
  // The tree changed: every popup draws again once its previous frame is answered.
  void changed();
  // Draws and presents each configured popup that has changed since its last frame.
  // Draws the popups that changed, and publishes their accessibility.
  void present(const TextInputEdit& edit, const std::string& focus);
  // A compositor dismissal (PopupDone): the view's onDismiss, and the surface is destroyed.
  void dismissed(const OxyOSEvent& event);
  // A key press or committed text sent to a popup's surface: the view's onKey, one event per key
  // with its W3C `KeyboardEvent.key` name (ArrowDown, Enter, Escape, …, or the character).
  void key(const OxyOSEvent& event);

  // The surface a view is drawn on: its popup's, or the toplevel's.
  uint32_t surfaceOf(facebook::react::Tag view) const;
  // What a popup surface shows, for pointer input; nullopt for any other surface.
  std::optional<SurfaceContent> content(uint32_t surface) const;
  // Whether a popup surface was closed by the application (its view went away), not by the
  // compositor: the keyboard it had goes back to its parent next, so its leaving is no loss.
  bool closedByApplication(uint32_t surface) const { return closed_.count(surface) != 0; }

 private:
  struct Popup {
    uint32_t surface = 0;
    OxyOSPositioner placed{};  // the positioner last sent
    bool dirty = true;
    // The configure its last frame was drawn for: a new one (a reposition that resized it, or a
    // scale change) needs a new frame, and the compositor discards one drawn for an older size.
    uint64_t drawnFor = 0;
    bool dismissed = false;  // closed by the compositor; the view is still mounted
  };
  // The anchor's surface, and its frame there.
  std::optional<std::pair<uint32_t, facebook::react::Rect>> anchored(
      const facebook::react::OxyOSPopupViewProps& props) const;
  const ViewTree& tree_;
  Window& window_;
  AccessibilityBridge& accessibility_;
  Parent parent_;
  std::map<facebook::react::Tag, Popup> popups_;
  std::set<uint32_t> closed_;
};

}  // namespace oxyos
