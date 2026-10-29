// Pointer input for the host's window: compositor pointer events become React Native touch
// events on the view under the pointer, as Android's touch handling delivers them, so React
// Native's responder system and Pressability work unchanged. Scrolls move the nearest scroll
// view that can scroll, which owns its offset as a native scroll view does.
#pragma once

#include "renderer.h"

#include <oxyos_client.h>

#include <react/renderer/core/ReactPrimitives.h>

#include <chrono>
#include <functional>
#include <optional>
#include <unordered_map>
#include <vector>

namespace oxyos {

// What a protocol surface shows: the window shows the root (popup subtrees and shell regions
// excluded); a popup shows its popup view's children, a shell surface its region's (`popup` for
// both). `origin` is where the surface's own origin sits in root coordinates, the space React
// Native measures in.
struct SurfaceContent {
  uint32_t surface = 0;
  facebook::react::Tag view = -1;
  bool popup = false;
  facebook::react::Point origin{};
};

// The deepest view under a surface-local logical point that takes touches, honouring
// `pointerEvents`, clipping and scroll offsets as they are drawn. -1 if none does.
facebook::react::Tag hitTest(const ViewTree& tree, const SurfaceContent& content, float x, float y);
// The deepest view under the point for react-native-gesture-handler, as its Android orchestrator
// traverses: as hitTest, but a view all but transparent (opacity under 0.1) is passed over, and a
// view `reaches` counts is under the point though outside its bounds (a handler's hit slop).
facebook::react::Tag gestureHitTest(const ViewTree& tree, const SurfaceContent& content, float x, float y,
                                    const std::function<bool(facebook::react::Tag)>& reaches);

class PointerInput {
 public:
  using Surfaces = std::function<std::optional<SurfaceContent>(uint32_t surface)>;
  PointerInput(const ViewTree& tree, Surfaces surfaces) : tree_(tree), surfaces_(std::move(surfaces)) {}

  // Handles a compositor pointer or scroll event. The primary button starts, moves and ends one
  // touch; a cancel or the pointer leaving cancels it.
  void handle(const OxyOSEvent& event);
  // Scrolls a text input's text by (dx, dy) if it can still move that way (textinput.h); true
  // if it moved. A text input under the pointer takes the wheel before the scroll views around it.
  void scrollsText(std::function<bool(facebook::react::Tag, float dx, float dy)> scrollText) {
    scrollText_ = std::move(scrollText);
  }
  // The touch's start, moves, end and cancel (0 to 3) also go to gesture handlers
  // (gesture-handler.h), after React Native, as Android's root view passes them on.
  void feedsGestures(std::function<void(int phase, const SurfaceContent& content, float x, float y)> feed) {
    gestures_ = std::move(feed);
  }
  // A gesture handler activated and cancels React Native's touch: it is sent touchcancel and
  // nothing more of this touch, as Android's JSTouchDispatcher does when a native gesture starts.
  void cancelResponder();

 private:
  void scroll(float dx, float dy);
  // Moves the hover to the views under (x, y) of `content` (none for nullopt): pointer leave
  // events for the views left, innermost first, then enter events for the views entered,
  // outermost first, as W3C pointer events do. Pressability's hover is built on them.
  void hover(const std::optional<SurfaceContent>& content, float x, float y);

  enum class Phase { Start, Move, End, Cancel };
  void dispatch(Phase phase, float x, float y);

  const ViewTree& tree_;
  Surfaces surfaces_;
  std::function<bool(facebook::react::Tag, float, float)> scrollText_;
  std::function<void(int, const SurfaceContent&, float, float)> gestures_;
  bool responderCancelled_ = false;  // React Native has let go of the current touch
  SurfaceContent content_{};  // the surface the pointer is over
  std::vector<facebook::react::Tag> hovered_;  // views under the pointer, root first
  uint32_t hoveredSurface_ = 0;
  bool down_ = false;
  facebook::react::Tag target_ = -1;
  int identifier_ = 0;
  float x_ = 0, y_ = 0;  // last pointer position, surface-local logical
  // Offsets this host gave each scroll view; the state commit that carries one may still be
  // pending when the next scroll arrives.
  std::unordered_map<facebook::react::Tag, facebook::react::Point> offsets_;
  std::chrono::steady_clock::time_point epoch_ = std::chrono::steady_clock::now();
};

}  // namespace oxyos
