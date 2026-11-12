// The shell's regions in surfaces of their own (docs/adr/0009-session-shell.mdx). The shell is
// one React tree whose root is a background shell surface; a region wrapped in the
// `OxyOSShellSurface` host component (the shell's `ShellSurface` on OxyOS) is drawn in a panel at
// an output edge or in an overlay, and left out of the root's drawing, as Bloom's popups are.
//
// Each region keeps its place in the shell's layout. A panel is the strip of the output from its
// edge to the far side of its region (kept out of the work area when `exclusive`), an overlay the
// whole output; the region is drawn at its own place in it, and the surface takes input only over
// the region, so the rest is transparent and lets presses through.
//
// A "candidates" region is the candidate window (docs/adr/0041-input-methods.mdx): the shell's
// candidates surface, exactly the region's size (outward to whole pixels), which the compositor
// places at the caret of the field an input method serves, never where the shell's layout put it.
// Its size follows the region's in place (the compositor shows the frame of the old size until
// one of the new is presented; a compositor before protocol version 21 has it made anew), and the
// shell keeps the region mounted, holding a point while it has no list, so the screen reader holds
// the same window from one list to the next.
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
#include <set>
#include <string>
#include <vector>

namespace facebook::react {

extern const char OxyOSShellSurfaceComponentName[];

class OxyOSShellSurfaceProps final : public ViewProps {
 public:
  OxyOSShellSurfaceProps() = default;
  OxyOSShellSurfaceProps(const PropsParserContext& context, const OxyOSShellSurfaceProps& sourceProps,
                         const RawProps& rawProps);

  // "panel", "overlay", "backdrop", "notification", "lock" (docs/adr/0010-lock-screen.mdx) or
  // "candidates" (docs/adr/0041-input-methods.mdx)
  std::string kind{"panel"};
  // A panel's: "top", "bottom", "left" or "right"; a notification at the right is a banner
  // (docs/adr/0026-notifications.mdx), any other a bezel.
  std::string edge{"top"};
  bool exclusive{false};
  // Whether the region asks for the keyboard (the top bar from the keyboard, docs/adr/0024-system-
  // shortcuts.mdx): the host asks the compositor for its surface's focus, citing the latest input.
  bool keyboard{false};
  // Where the keyboard's focus goes once the region has the keyboard (keyboard-focus.h): "first",
  // its first control, or the testID of one; each new `focusRequest` asks again.
  std::string focusTarget{};
  int focusRequest{0};
  // Whether its surface is left out of every screenshot (the screenshot interface itself and its
  // thumbnail, docs/adr/0024-system-shortcuts.mdx, decision 10).
  bool excludeFromCapture{false};
  // Whether the windows its views show are moved into them (App Exposé, docs/adr/0045-app-expose.mdx):
  // drawn there alone while it shows, flying from where they are as it arrives and back as it goes.
  bool movesWindows{false};
  // Which view a touchpad's gesture drives it is (docs/adr/0046-interactive-gestures.mdx): "" none,
  // "mission-control" or "app-expose" (swiped up and down) or "launchpad" (the thumb and three
  // fingers pinched): the gesture draws it as far as the fingers have it.
  std::string gesture;
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

// Where windows minimize into (docs/adr/0011-transitions.mdx, the genie): the view's frame, in the
// shell surface it is drawn in, is where each window it names (`windows`, their numbers) goes when
// minimized and comes back out of when restored, as a Mac's Dock tells the window server its
// icons' rectangles. The view is drawn as any other; what it holds (the dock's icon) is its own.
extern const char OxyOSMinimizeTargetComponentName[];

class OxyOSMinimizeTargetProps final : public ViewProps {
 public:
  OxyOSMinimizeTargetProps() = default;
  OxyOSMinimizeTargetProps(const PropsParserContext& context, const OxyOSMinimizeTargetProps& sourceProps,
                           const RawProps& rawProps);

  std::vector<int> windows;  // 0 or less names none
};

using OxyOSMinimizeTargetShadowNode =
    ConcreteViewShadowNode<OxyOSMinimizeTargetComponentName, OxyOSMinimizeTargetProps>;
using OxyOSMinimizeTargetComponentDescriptor = ConcreteComponentDescriptor<OxyOSMinimizeTargetShadowNode>;

// What takes the pointer's scrolls no scroll view under it takes (the dock's icons, whose scroll up
// opens App Exposé, docs/adr/0045-app-expose.mdx): as Android's View takes the ACTION_SCROLL
// generic motion events nothing within it handled. Each is told to JavaScript with `target`, the
// number JavaScript gave it. Drawn and laid out as any view.
extern const char OxyOSScrollTargetComponentName[];

class OxyOSScrollTargetProps final : public ViewProps {
 public:
  OxyOSScrollTargetProps() = default;
  OxyOSScrollTargetProps(const PropsParserContext& context, const OxyOSScrollTargetProps& sourceProps,
                         const RawProps& rawProps);

  int target{0};
};

using OxyOSScrollTargetShadowNode = ConcreteViewShadowNode<OxyOSScrollTargetComponentName, OxyOSScrollTargetProps>;
using OxyOSScrollTargetComponentDescriptor = ConcreteComponentDescriptor<OxyOSScrollTargetShadowNode>;

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
  // Whether a region's last frame had a ring spinning (progress-bar.h); turn(): each such region
  // draws again (true if one is ready to).
  bool spinning() const;
  bool turn();
  // Every region draws again whole (another painter took over).
  void redrawWhole();
  // Draws and presents the regions that changed, and publishes their accessibility.
  void present(const TextInputEdit& edit, const std::string& focus);

  // The surface a view is drawn on, if it is inside a region.
  std::optional<uint32_t> surfaceOf(facebook::react::Tag view) const;
  // The lock screen's surface, while a "lock" region is open.
  std::optional<uint32_t> lockSurface() const;
  // Whether this client closed `surface` itself (a region gone or made again: Control Center
  // closed by its Escape), which keeps the keyboard as far as the client knows until the
  // compositor gives it to the surface next in line, as popups.h's closedByApplication.
  bool closedByApplication(uint32_t surface) const { return closed_.count(surface) != 0; }
  // The surface of the region asking for the keyboard (its `keyboard` prop), once it is open.
  std::optional<uint32_t> keyboardRegion() const;
  // The keyboard region's request for the keyboard's focus (its `focusTarget`, `focusRequest`).
  struct FocusRequest {
    uint32_t surface = 0;
    facebook::react::Tag region = -1;
    std::string target;
    int request = 0;
  };
  std::optional<FocusRequest> focusRequest() const;
  // What a region's surface shows, for input; nullopt for any other surface.
  std::optional<SurfaceContent> content(uint32_t surface) const;

 private:
  // A lock region's kind, beside OXYOS_SHELL_*: a lock surface, not a shell surface.
  static constexpr uint8_t kLock = 0;
  // A candidates region's: the candidates surface, placed by the compositor at the caret.
  static constexpr uint8_t kCandidates = 255;
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
    // Where the windows it names minimize into, as last sent to the compositor (ADR 0011).
    std::vector<RenderStats::MinimizeTarget> targets;
    // Whether the compositor was told to leave its surface out of captures (ADR 0024), and to
    // move the windows it shows into their views (ADR 0045).
    bool excluded = false;
    bool moves = false;
    // The view a swipe drives it is, as last told (OXYOS_GESTURE_OVERLAY_*).
    uint8_t gesture = 0;
    // A ring spun in its last frame (progress-bar.h).
    bool spins = false;
  };
  // The region asking for the keyboard: of several, the newest.
  std::optional<facebook::react::Tag> keyboardTag() const;
  // Tells the compositor a region's `excludeFromCapture`, `movesWindows` and `gesture` when they
  // differ from what it was told.
  void tellFlags(Region& region, const facebook::react::OxyOSShellSurfaceProps& props);
  // Sends a region's windows shown live if they changed since last sent; true if they did.
  bool showWindows(Region& region, const std::vector<RenderStats::WindowView>& windows);
  // Sends where a region's windows minimize into if it changed since last sent (held from the
  // region's next present); true if it did.
  bool placeMinimizeTargets(Region& region, const std::vector<RenderStats::MinimizeTarget>& targets);
  const ViewTree& tree_;
  Window& window_;
  AccessibilityBridge& accessibility_;
  std::map<facebook::react::Tag, Region> regions_;
  // The surfaces it closed (ids are never used again).
  std::set<uint32_t> closed_;
};

}  // namespace oxyos
