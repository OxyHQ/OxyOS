#include "damage.h"

#include "svg.h"

#include <react/renderer/components/scrollview/ScrollViewState.h>
#include <react/renderer/core/ConcreteState.h>

#include <algorithm>
#include <cstdio>
#include <cstdlib>

using namespace facebook::react;

namespace oxyos {

namespace {

// Every view the window draws, where it draws it: the traversal `drawView` makes, popup subtrees
// left out (they are drawn in their own surfaces), and an SVG view's elements too (they are its
// picture: what changes in them changes the picture).
void collect(const ViewTree& tree, Tag tag, Float ox, Float oy, std::unordered_map<Tag, Rect>& at,
             std::unordered_map<Tag, const View*>& views) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  const ShadowView& view = it->second.view;
  if (view.componentName && ownSurface(view.componentName)) return;
  const auto& frame = view.layoutMetrics.frame;
  Rect rect{{ox + frame.origin.x, oy + frame.origin.y}, frame.size};
  // A modal is drawn at the window's origin, wherever it is in the tree.
  if (view.componentName && isModal(view.componentName)) rect.origin = Point{0, 0};
  at[tag] = rect;
  views[tag] = &it->second;
  if (view.componentName && isSvgView(view.componentName)) return;
  Float childOx = rect.origin.x, childOy = rect.origin.y;
  if (view.componentName && std::string(view.componentName) == "ScrollView" && view.state) {
    const auto& state = static_cast<const ConcreteState<ScrollViewState>&>(*view.state);
    childOx -= state.getData().contentOffset.x;
    childOy -= state.getData().contentOffset.y;
  }
  for (Tag child : it->second.children) collect(tree, child, childOx, childOy, at, views);
}

Rect bounding(const std::vector<Rect>& rects) {
  Float left = rects[0].getMinX(), top = rects[0].getMinY();
  Float right = rects[0].getMaxX(), bottom = rects[0].getMaxY();
  for (const auto& rect : rects) {
    left = std::min(left, rect.getMinX());
    top = std::min(top, rect.getMinY());
    right = std::max(right, rect.getMaxX());
    bottom = std::max(bottom, rect.getMaxY());
  }
  return Rect{{left, top}, {right - left, bottom - top}};
}

}  // namespace

std::optional<std::vector<Rect>> WindowDamage::update(const ViewTree& tree, const Display& display,
                                                      const TextInputEdit& edit) {
  std::unordered_map<Tag, Rect> at;
  std::unordered_map<Tag, const View*> views;
  collect(tree, tree.root(), 0, 0, at, views);
  Views now;
  for (const auto& [tag, rect] : at) {
    const View& view = *views[tag];
    bool svg = view.view.componentName && isSvgView(view.view.componentName);
    now[tag] = Seen{view.view, rect, view.children, svg ? svgPicture(tree, tag, display.scale) : view.picture};
  }

  bool same = display_ && display_->width == display.width && display_->height == display.height &&
              display_->scale == display.scale;
  std::vector<Rect> damage;
  // A view and everything under it, as `views` holds them.
  auto subtree = [&](const Views& views, Tag tag, auto& self) -> void {
    auto it = views.find(tag);
    if (it == views.end()) return;
    damage.push_back(it->second.at);
    for (Tag child : it->second.children) self(views, child, self);
  };
  if (same) {
    for (const auto& [tag, seen] : now) {
      auto old = seen_.find(tag);
      if (old == seen_.end()) {
        damage.push_back(seen.at);
        continue;
      }
      const auto& then = old->second;
      // A new picture changes the view's own drawing, under its children.
      if (then.picture != seen.picture) damage.push_back(seen.at);
      bool props = !drawsTheSame(then.view, seen.view);
      bool state = then.view.state != seen.view.state;
      // OXYOS_DAMAGE_TRACE: what drew a subtree again, and why.
      static const bool trace = std::getenv("OXYOS_DAMAGE_TRACE") != nullptr;
      if (props || state) {
        if (trace) {
          std::fprintf(stderr, "[damage] %s tag=%d %.0fx%.0f:%s%s\n",
                       seen.view.componentName ? seen.view.componentName : "?", tag, seen.at.size.width,
                       seen.at.size.height, props ? " props" : "", state ? " state" : "");
        }
        subtree(seen_, tag, subtree);
        subtree(now, tag, subtree);
        continue;
      }
      if (then.children != seen.children) {
        // Children added or gone are damaged where they are or were, as every new or gone view
        // is (a popup view's not at all: it is not drawn in the window). What else a new list
        // of children changes is the order the remaining ones are drawn in: if it changed, they
        // are drawn again, subtrees and all. The view's own drawing does not depend on them.
        auto kept = [](const std::vector<Tag>& children, const std::vector<Tag>& others) {
          std::vector<Tag> common;
          for (Tag child : children) {
            if (std::find(others.begin(), others.end(), child) != others.end()) common.push_back(child);
          }
          return common;
        };
        auto before = kept(then.children, seen.children), after = kept(seen.children, then.children);
        if (before != after) {
          if (trace) {
            std::fprintf(stderr, "[damage] %s tag=%d: children reordered\n",
                         seen.view.componentName ? seen.view.componentName : "?", tag);
          }
          for (Tag child : after) {
            subtree(seen_, child, subtree);
            subtree(now, child, subtree);
          }
        }
      }
      if (!(then.at == seen.at) || !(then.view.layoutMetrics == seen.view.layoutMetrics)) {
        // A new frame alone: its own background and border, and the clip of its children, lie
        // in its old and new rectangles; each child is compared where it is drawn.
        damage.push_back(old->second.at);
        damage.push_back(seen.at);
      }
    }
    for (const auto& [tag, seen] : seen_) {
      if (!now.count(tag)) damage.push_back(seen.at);
    }
    if (!(edit == edit_)) {
      std::vector<Tag> changed{edit_.tag, edit.tag};
      // An input whose text scrolled, focused or not (the wheel scrolls any input under it).
      auto scroll = [](const TextInputEdit& edit, Tag tag) {
        auto it = edit.scrolls.find(tag);
        return it == edit.scrolls.end() ? Point{} : it->second;
      };
      for (const auto& scrolls : {edit_.scrolls, edit.scrolls}) {
        for (const auto& entry : scrolls) {
          if (!(scroll(edit_, entry.first) == scroll(edit, entry.first))) changed.push_back(entry.first);
        }
      }
      for (Tag tag : changed) {
        if (auto it = now.find(tag); it != now.end()) damage.push_back(it->second.at);
        if (auto it = seen_.find(tag); it != seen_.end()) damage.push_back(it->second.at);
      }
    }
  }
  seen_ = std::move(now);
  display_ = display;
  edit_ = edit;
  if (!same) return std::nullopt;
  // Inside the window, without empty rectangles or repeats.
  Rect window{{0, 0}, {display.width, display.height}};
  std::vector<Rect> kept;
  for (const auto& rect : damage) {
    Rect inside = Rect::intersect(rect, window);
    if (inside.size.width <= 0 || inside.size.height <= 0) continue;
    if (std::find(kept.begin(), kept.end(), inside) == kept.end()) kept.push_back(inside);
  }
  if (kept.size() > kMaxRects) kept = {bounding(kept)};
  // OXYOS_DAMAGE_NEGATIVE_CONTROL: every rectangle 2 pixels short on each side, so that the check
  // (OXYOS_HOST_DAMAGE_CHECK) can be seen to fail. Never set outside that test.
  if (std::getenv("OXYOS_DAMAGE_NEGATIVE_CONTROL")) {
    for (auto& rect : kept) {
      rect = Rect{{rect.origin.x + 2, rect.origin.y + 2},
                  {std::max<Float>(0, rect.size.width - 4), std::max<Float>(0, rect.size.height - 4)}};
    }
  }
  return kept;
}

}  // namespace oxyos
