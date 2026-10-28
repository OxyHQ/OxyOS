#pragma once
// What changed in the window between two frames (docs/adr/0005-damage.mdx): the host draws only
// that (renderer.h's drawDamaged) and tells the compositor (oxy_client_present_damaged).

#include "renderer.h"

#include <optional>
#include <unordered_map>
#include <vector>

namespace oxy {

class WindowDamage {
 public:
  // At most this many rectangles; more become the one holding them all.
  static constexpr size_t kMaxRects = 16;

  // What changed in the window since the last call, in window-local logical pixels:
  // - a view that is new or gone, where it is or was;
  // - a view that moved or resized, both places (each child is compared where it is drawn);
  // - a view drawn with other props (renderer.h's drawsTheSame) or a new state, with its whole
  //   subtree both before and after (a view draws its subtree with its opacity and clip);
  // - children drawn in a new order, with their subtrees (children added or removed are new or
  //   gone views; a popup view is drawn elsewhere);
  // - the text input whose caret, selection or composition moved.
  // Nothing (`std::nullopt`) means everything: the first frame, or a new size or scale.
  std::optional<std::vector<facebook::react::Rect>> update(const ViewTree& tree, const Display& display,
                                                           const TextInputEdit& edit);

 private:
  struct Seen {
    facebook::react::ShadowView view;
    facebook::react::Rect at;  // window-local, as drawn (scroll offsets applied)
    std::vector<facebook::react::Tag> children;
  };
  using Views = std::unordered_map<facebook::react::Tag, Seen>;
  Views seen_;
  std::optional<Display> display_;
  TextInputEdit edit_;
};

}  // namespace oxy
