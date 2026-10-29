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

  std::string kind{"panel"};  // "panel", "overlay" or "lock" (docs/adr/0010-lock-screen.mdx)
  std::string edge{"top"};    // a panel's: "top", "bottom", "left" or "right"
  bool exclusive{false};
};

using OxyOSShellSurfaceShadowNode = ConcreteViewShadowNode<OxyOSShellSurfaceComponentName, OxyOSShellSurfaceProps>;
using OxyOSShellSurfaceComponentDescriptor = ConcreteComponentDescriptor<OxyOSShellSurfaceShadowNode>;

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
  // Draws and presents the regions that changed, and publishes their accessibility.
  void present(const TextInputEdit& edit, const std::string& focus);

  // The surface a view is drawn on, if it is inside a region.
  std::optional<uint32_t> surfaceOf(facebook::react::Tag view) const;
  // The lock screen's surface, while a "lock" region is open.
  std::optional<uint32_t> lockSurface() const;
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
    uint64_t drawnFor = 0;
    // The frame as last drawn and what changed since: the next draws only that (damage.h).
    Canvas canvas;
    WindowDamage damage;
  };
  const ViewTree& tree_;
  Window& window_;
  AccessibilityBridge& accessibility_;
  std::map<facebook::react::Tag, Region> regions_;
};

}  // namespace oxyos
