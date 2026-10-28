// Popups for Bloom: the `OxyPopupView` host component (sample/oxyos/platform-surface.js renders it
// as the OxyOS adapter's PopupSurface) and the compositor popup each mounted one becomes.
//
// The popup view stays in the app's React tree, like React Native's Modal: providers and state
// reach its content as they reach any child. The host leaves its subtree out of the window and
// draws it in a protocol popup of its own, placed by the compositor from the anchor's frame and
// Bloom's placement. The compositor's dismissal is sent back as the view's `onDismiss`.
#pragma once

#include "input.h"
#include "renderer.h"
#include "window.h"

#include <react/renderer/components/view/ConcreteViewShadowNode.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

#include <map>
#include <optional>
#include <string>

namespace facebook::react {

extern const char OxyPopupViewComponentName[];

class OxyPopupViewProps final : public ViewProps {
 public:
  OxyPopupViewProps() = default;
  OxyPopupViewProps(const PropsParserContext& context, const OxyPopupViewProps& sourceProps, const RawProps& rawProps);

  // The anchor view's tag, and Bloom's placement against it.
  Tag anchorTag{-1};
  std::string side{"bottom"};
  std::string align{"start"};
  Float sideOffset{0};
  Float alignOffset{0};
};

using OxyPopupViewShadowNode = ConcreteViewShadowNode<OxyPopupViewComponentName, OxyPopupViewProps>;
using OxyPopupViewComponentDescriptor = ConcreteComponentDescriptor<OxyPopupViewShadowNode>;

}  // namespace facebook::react

namespace oxy {

// Bloom's placement as the protocol's positioner (docs/adr/0002): flip on the side axis, slide on
// both. `anchor` is the anchor's window-local frame.
OxyPositioner positioner(const facebook::react::OxyPopupViewProps& props, facebook::react::Rect anchor,
                         facebook::react::Size size);

class PopupHost {
 public:
  PopupHost(const ViewTree& tree, Window& window) : tree_(tree), window_(window) {}

  // Opens a popup for each newly mounted popup view (grabbed with `serial`, the input that opened
  // it) and destroys the popups whose view went away.
  void sync(uint64_t serial);
  // The tree changed: every popup draws again once its previous frame is answered.
  void changed();
  // Draws and presents each configured popup that has changed since its last frame.
  void present(facebook::react::Tag caret, const std::string& focus);
  // A compositor dismissal (PopupDone): the view's onDismiss, and the surface is destroyed.
  void dismissed(const OxyEvent& event);

  // The surface a view is drawn on: its popup's, or the toplevel's.
  uint32_t surfaceOf(facebook::react::Tag view) const;
  // What a popup surface shows, for pointer input; nullopt for any other surface.
  std::optional<SurfaceContent> content(uint32_t surface) const;

 private:
  struct Popup {
    uint32_t surface = 0;
    bool dirty = true;
    bool dismissed = false;  // closed by the compositor; the view is still mounted
  };
  const ViewTree& tree_;
  Window& window_;
  std::map<facebook::react::Tag, Popup> popups_;
  uint32_t nextSurface_ = Window::kToplevel + 1;
};

}  // namespace oxy
