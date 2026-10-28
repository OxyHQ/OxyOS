// Pointer input for the host's window: compositor pointer events become React Native touch
// events on the view under the pointer, as Android's touch handling delivers them. React
// Native's responder system and Pressability then work unchanged.
#pragma once

#include "renderer.h"

#include <oxy_client.h>

#include <react/renderer/core/ReactPrimitives.h>

#include <chrono>

namespace oxy {

// The deepest view under a window-local logical point that takes touches, honouring
// `pointerEvents`, clipping and scroll offsets as they are drawn. -1 if none does.
facebook::react::Tag hitTest(const ViewTree& tree, float x, float y);

class PointerInput {
 public:
  explicit PointerInput(const ViewTree& tree) : tree_(tree) {}

  // Handles a compositor pointer event: the primary button starts, moves and ends one touch;
  // a cancel or the pointer leaving cancels it.
  void handle(const OxyEvent& event);

 private:
  enum class Phase { Start, Move, End, Cancel };
  void dispatch(Phase phase, float x, float y);

  const ViewTree& tree_;
  bool down_ = false;
  facebook::react::Tag target_ = -1;
  int identifier_ = 0;
  std::chrono::steady_clock::time_point epoch_ = std::chrono::steady_clock::now();
};

}  // namespace oxy
