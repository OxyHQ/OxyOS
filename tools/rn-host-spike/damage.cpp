#include "damage.h"

#include "expo-blur.h"
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
// Each view's rectangle where it is drawn: its frame with what it draws outside it (renderer.h's
// inkBounds: outset shadows, outline, filter), `parent` mapping its parent's coordinates to the
// window's. A transformed view's is the bounding box of that, transformed.
void collect(const ViewTree& tree, Tag tag, const Affine& parent, std::unordered_map<Tag, Rect>& at,
             std::unordered_map<Tag, const View*>& views) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  const ShadowView& view = it->second.view;
  if (view.componentName && ownSurface(view.componentName)) return;
  // A modal is drawn at the window's origin, wherever it is in the tree.
  bool modal = view.componentName && isModal(view.componentName);
  Affine map = modal ? Affine{} : parent.after(viewTransform(view));
  at[tag] = map.bounds(inkBounds(view));
  views[tag] = &it->second;
  if (view.componentName && isSvgView(view.componentName)) return;
  if (view.componentName && std::string(view.componentName) == "ScrollView" && view.state) {
    const auto& state = static_cast<const ConcreteState<ScrollViewState>&>(*view.state);
    map = map.after(Affine::translation(-state.getData().contentOffset.x, -state.getData().contentOffset.y));
  }
  for (Tag child : it->second.children) collect(tree, child, map, at, views);
}

// The parts of `a` outside `b`: up to four bands.
void outside(const Rect& a, const Rect& b, std::vector<Rect>& out) {
  Rect common = Rect::intersect(a, b);
  if (common.size.width <= 0 || common.size.height <= 0) {
    out.push_back(a);
    return;
  }
  auto band = [&](Float x0, Float y0, Float x1, Float y1) {
    if (x1 > x0 && y1 > y0) out.push_back(Rect{{x0, y0}, {x1 - x0, y1 - y0}});
  };
  band(a.getMinX(), a.getMinY(), a.getMaxX(), common.getMinY());
  band(a.getMinX(), common.getMaxY(), a.getMaxX(), a.getMaxY());
  band(a.getMinX(), common.getMinY(), common.getMinX(), common.getMaxY());
  band(common.getMaxX(), common.getMinY(), a.getMaxX(), common.getMaxY());
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
  collect(tree, tree.root(), Affine{}, at, views);
  return compare(tree, at, views, display, edit);
}

std::optional<std::vector<Rect>> WindowDamage::update(const ViewTree& tree, Tag region, Point origin,
                                                      const Display& display, const TextInputEdit& edit) {
  std::unordered_map<Tag, Rect> at;
  std::unordered_map<Tag, const View*> views;
  auto it = tree.views().find(region);
  if (it != tree.views().end()) {
    for (Tag child : it->second.children) collect(tree, child, Affine::translation(origin.x, origin.y), at, views);
  }
  return compare(tree, at, views, display, edit);
}

std::optional<std::vector<Rect>> WindowDamage::compare(const ViewTree& tree, const std::unordered_map<Tag, Rect>& at,
                                                       const std::unordered_map<Tag, const View*>& views,
                                                       const Display& display, const TextInputEdit& edit) {
  Views now;
  for (const auto& [tag, rect] : at) {
    const View& view = *views.at(tag);
    bool svg = view.view.componentName && isSvgView(view.view.componentName);
    now[tag] = Seen{view.view, rect, view.children, view.picture, svg ? svgPicture(tree, tag) : nullptr};
  }

  // A window resized at the same scale is compared view by view as at the same size: what the
  // frame before has where both sizes overlap stays (renderer.h's drawDamaged).
  bool same = display_ && display_->scale == display.scale;
  bool resized = same && (display_->width != display.width || display_->height != display.height);
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
      if (then.picture != seen.picture || then.drawing != seen.drawing) damage.push_back(seen.at);
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
        // in its old and new rectangles; each child is compared where it is drawn. A plain fill
        // resized in place (a window's panels as it resizes) changes only where it gained or
        // lost area.
        if (then.at.origin == seen.at.origin && drawsFlat(then.view) && drawsFlat(seen.view)) {
          outside(then.at, seen.at, damage);
          outside(seen.at, then.at, damage);
        } else {
          damage.push_back(old->second.at);
          damage.push_back(seen.at);
        }
      }
    }
    for (const auto& [tag, seen] : seen_) {
      if (!now.count(tag)) damage.push_back(seen.at);
    }
  }
  std::vector<std::pair<Tag, ModalPresentation>> presented;
  for (Tag modal : modals(tree)) presented.emplace_back(modal, tree.presentation(modal));
  // A modal is laid out at the window's size and a leaving one's frame is the window's: resized,
  // the window is drawn again whole while any shows.
  if (same && (presented != presented_ || tree.leaving() != leaving_ ||
               (resized && (!presented.empty() || !tree.leaving().empty())))) {
    damage.push_back(Rect{{0, 0}, {display.width, display.height}});
  }
  presented_ = std::move(presented);
  leaving_ = tree.leaving();
  // A scroll view stretched past its end (overscroll) draws its content anew wherever it is, as the
  // stretch changes.
  if (same) {
    const std::map<Tag, Point>* both[] = {&stretched_, &tree.stretches()};
    for (const auto* stretches : both) {
      for (const auto& [tag, amount] : *stretches) {
        auto before = stretched_.find(tag);
        auto after = tree.stretches().find(tag);
        Point was = before == stretched_.end() ? Point{} : before->second;
        Point is = after == tree.stretches().end() ? Point{} : after->second;
        if (was == is) continue;
        if (auto it = now.find(tag); it != now.end()) damage.push_back(it->second.at);
      }
    }
  }
  // A change inside a stretched scroll view is drawn where the stretch puts it: the whole view.
  if (same) {
    std::vector<Rect> within;
    for (const auto& [tag, amount] : tree.stretches()) {
      auto it = now.find(tag);
      if (it == now.end()) continue;
      for (const auto& rect : damage) {
        Rect common = Rect::intersect(rect, it->second.at);
        if (common.size.width > 0 && common.size.height > 0) {
          within.push_back(it->second.at);
          break;
        }
      }
    }
    damage.insert(damage.end(), within.begin(), within.end());
  }
  stretched_ = tree.stretches();
  if (same) {
    if (!(edit == edit_)) {
      std::vector<Tag> changed{edit_.tag, edit.tag, edit_.dropTag, edit.dropTag};
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
      // The keyboard's focus ring, around its control.
      auto ringed = [](Rect at) {
        return Rect{{at.origin.x - kFocusRingReach, at.origin.y - kFocusRingReach},
                    {at.size.width + 2 * kFocusRingReach, at.size.height + 2 * kFocusRingReach}};
      };
      // A ring that moved, or that changed colour with the accent.
      if (edit_.focusRing != edit.focusRing || edit_.focusRingColor != edit.focusRingColor) {
        for (Tag tag : {edit_.focusRing, edit.focusRing}) {
          if (auto it = now.find(tag); it != now.end()) damage.push_back(ringed(it->second.at));
          if (auto it = seen_.find(tag); it != seen_.end()) damage.push_back(ringed(it->second.at));
        }
      }
    }
  }
  // A ring around a control that moved, or whose drawing changed, goes with it.
  if (same && edit.focusRing >= 0 && edit.focusRing == edit_.focusRing) {
    auto now_ = now.find(edit.focusRing);
    auto then = seen_.find(edit.focusRing);
    if (now_ != now.end() && then != seen_.end() && !(now_->second.at == then->second.at)) {
      for (Rect at : {now_->second.at, then->second.at}) {
        damage.push_back(Rect{{at.origin.x - kFocusRingReach, at.origin.y - kFocusRingReach},
                              {at.size.width + 2 * kFocusRingReach, at.size.height + 2 * kFocusRingReach}});
      }
    }
  }
  if (same) {
    // A blur view with a target (expo-blur.h) reads the target three of its radii around each of
    // its pixels: a change that near it changes it too.
    std::vector<Rect> blurred;
    for (const auto& [tag, seen] : now) {
      auto blur = expoBlurTarget(seen.view);
      if (!blur) continue;
      Float reach = 3 * blur->radius;
      Rect around{{seen.at.origin.x - reach, seen.at.origin.y - reach},
                  {seen.at.size.width + 2 * reach, seen.at.size.height + 2 * reach}};
      for (const auto& rect : damage) {
        Rect common = Rect::intersect(rect, around);
        if (common.size.width > 0 && common.size.height > 0) {
          blurred.push_back(seen.at);
          break;
        }
      }
    }
    damage.insert(damage.end(), blurred.begin(), blurred.end());
  }
  seen_ = std::move(now);
  Display previous = display_.value_or(display);
  display_ = display;
  edit_ = edit;
  if (!same) return std::nullopt;
  // Inside the window, without empty rectangles or repeats; resized, with all it gained.
  Rect window{{0, 0}, {display.width, display.height}};
  if (resized) outside(window, Rect{{0, 0}, {previous.width, previous.height}}, damage);
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
