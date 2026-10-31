// The shell's regions in surfaces of their own (docs/adr/0009-session-shell.mdx). The shell is
// one React tree whose root is a background shell surface; a region wrapped in the
// `OxyOSShellSurface` host component (the shell's `ShellSurface` on OxyOS) is drawn in a panel at
// an output edge or in an overlay, and left out of the root's drawing, as Bloom's popups are.
//
// Each region keeps its place in the shell's layout. A panel is the strip of the output from its
// edge to the far side of its region (kept out of the work area when `exclusive`), an overlay the
// whole output; the region is drawn at its own place in it, and the surface takes input only over
// the region, so the rest is transparent and lets presses through.
#pragma once

#include "accessibility.h"
#include "input.h"
#include "damage.h"
#include "renderer.h"
#include "window.h"

#include <react/renderer/components/view/ConcreteViewShadowNode.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

#include <map>
#include <optional>
#include <string>

namespace facebook::react {

extern const char OxyOSShellSurfaceComponentName[];

class OxyOSShellSurfaceProps final : public ViewProps {
 public:
  OxyOSShellSurfaceProps() = default;
  OxyOSShellSurfaceProps(const PropsParserContext& context, const OxyOSShellSurfaceProps& sourceProps,
                         const RawProps& rawProps);

  // "panel", "overlay", "notification" or "lock" (docs/adr/0010-lock-screen.mdx)
  std::string kind{"panel"};
  // A panel's: "top", "bottom", "left" or "right"; a notification at the right is a banner
  // (docs/adr/0026-notifications.mdx), any other a bezel.
  std::string edge{"top"};
  bool exclusive{false};
  // Whether the region asks for the keyboard (the top bar from the keyboard, docs/adr/0024-system-
  // shortcuts.mdx): the host asks the compositor for its surface's focus, citing the latest input.
  bool keyboard{false};
  // Whether its surface is left out of every screenshot (the screenshot interface itself and its
  // thumbnail, docs/adr/0024-system-shortcuts.mdx, decision 10).
  bool excludeFromCapture{false};
};

using OxyOSShellSurfaceShadowNode = ConcreteViewShadowNode<OxyOSShellSurfaceComponentName, OxyOSShellSurfaceProps>;
using OxyOSShellSurfaceComponentDescriptor = ConcreteComponentDescriptor<OxyOSShellSurfaceShadowNode>;

// A window shown live (docs/adr/0022-window-views.mdx): the compositor draws the window numbered
// `window` over the shell surface the view is drawn in, fitted to the view's frame, cut to what
// clips it, with its top-left corner's radius and its opacity. The host draws nothing of it but
// the view's own background; the shell never receives the window's pixels.
extern const char OxyOSWindowViewComponentName[];

class OxyOSWindowViewProps final : public ViewProps {
 public:
  OxyOSWindowViewProps() = default;
  OxyOSWindowViewProps(const PropsParserContext& context, const OxyOSWindowViewProps& sourceProps,
                       const RawProps& rawProps);

  int window{0};  // 0 names none
};

using OxyOSWindowViewShadowNode = ConcreteViewShadowNode<OxyOSWindowViewComponentName, OxyOSWindowViewProps>;
using OxyOSWindowViewComponentDescriptor = ConcreteComponentDescriptor<OxyOSWindowViewShadowNode>;

}  // namespace facebook::react

namespace oxyos {

class ShellSurfaceHost {
 public:
  ShellSurfaceHost(const ViewTree& tree, Window& window, AccessibilityBridge& accessibility)
      : tree_(tree), window_(window), accessibility_(accessibility) {}

  // After a mount or a new root size: opens a surface for each new region, places again those
  // whose strip or input changed, and closes those whose region went away.
  void sync();
  // The tree changed: every region draws again once its previous frame is answered.
  void changed();
  // Every region draws again whole (another painter took over).
  void redrawWhole();
  // Draws and presents the regions that changed, and publishes their accessibility.
  void present(const TextInputEdit& edit, const std::string& focus);

  // The surface a view is drawn on, if it is inside a region.
  std::optional<uint32_t> surfaceOf(facebook::react::Tag view) const;
  // The lock screen's surface, while a "lock" region is open.
  std::optional<uint32_t> lockSurface() const;
  // The surface of the region asking for the keyboard (its `keyboard` prop), once it is open.
  std::optional<uint32_t> keyboardRegion() const;
  // What a region's surface shows, for input; nullopt for any other surface.
  std::optional<SurfaceContent> content(uint32_t surface) const;

 private:
  // A lock region's kind, beside OXYOS_SHELL_*: a lock surface, not a shell surface.
  static constexpr uint8_t kLock = 0;
  // Where a region's surface is: the strip in root coordinates, the region within it.
  struct Placement {
    uint8_t kind = 0, edge = 0;
    uint32_t exclusive = 0;
    facebook::react::Rect strip{};
    OxyOSRect input{};
    bool operator==(const Placement& other) const;
  };
  std::optional<Placement> place(facebook::react::Tag tag) const;
  struct Region {
    uint32_t surface = 0;
    Placement placed{};
    bool dirty = true;
    bool whole = false;
    uint64_t drawnFor = 0;
    // The frame as last drawn and what changed since: the next draws only that (damage.h).
    Canvas canvas;
    WindowDamage damage;
    // The windows it shows live, as last sent to the compositor (ADR 0022).
    std::vector<RenderStats::WindowView> windows;
    // Whether the compositor was told to leave its surface out of captures (ADR 0024).
    bool excluded = false;
  };
  // Tells the compositor a region's `excludeFromCapture` when it differs from what it was told.
  void exclude(Region& region, const facebook::react::OxyOSShellSurfaceProps& props);
  // Sends a region's windows shown live if they changed since last sent; true if they did.
  bool showWindows(Region& region, const std::vector<RenderStats::WindowView>& windows);
  const ViewTree& tree_;
  Window& window_;
  AccessibilityBridge& accessibility_;
  std::map<facebook::react::Tag, Region> regions_;
};

}  // namespace oxyos
