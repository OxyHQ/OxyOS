#include "modal.h"

#include "report.h"

#include <react/renderer/components/FBReactNativeSpec/EventEmitters.h>
#include <react/renderer/components/FBReactNativeSpec/Props.h>
#include <react/renderer/components/modal/ModalHostViewState.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/core/ConcreteState.h>

#include <algorithm>
#include <cmath>
#include <numbers>

using namespace facebook::react;

namespace oxyos {

namespace {

// The nearest view with a testID, the modal itself included, names it in the report.
std::string testIdOf(const ViewTree& tree, Tag tag) {
  auto path = ancestry(tree, tag);
  for (auto at = path.rbegin(); at != path.rend(); ++at) {
    auto props = std::dynamic_pointer_cast<const ViewProps>(tree.views().at(*at).view.props);
    if (props && !props->testId.empty()) return props->testId;
  }
  return "";
}

folly::dynamic within(const ViewTree& tree, Tag tag) {
  auto name = testIdOf(tree, tag);
  return name.empty() ? folly::dynamic::object() : folly::dynamic::object("within", name);
}

// The animation a modal's `animationType` asks for: none for `none`.
ModalHostViewAnimationType animationOf(const ShadowView& view) {
  auto props = std::dynamic_pointer_cast<const ModalHostViewProps>(view.props);
  return props ? props->animationType : ModalHostViewAnimationType::None;
}

}  // namespace

ModalPresentation ModalHost::presentation(const Animation& animation, bool in, float progress,
                                          const Display& display) const {
  if (animation.kind == Kind::Slide) {
    // A <translate> without an interpolator: Android's default, AccelerateDecelerateInterpolator.
    float eased = std::cos((progress + 1) * std::numbers::pi_v<float>) / 2 + 0.5f;
    return {(in ? 1 - eased : eased) * display.height, 1};
  }
  // @android:anim/accelerate_interpolator: AccelerateInterpolator, factor 1.
  float eased = progress * progress;
  return {0, in ? eased : 1 - eased};
}

void ModalHost::sync(const Display& display) {
  std::set<Tag> now;
  for (Tag tag : modals(tree_)) {
    const auto& view = tree_.views().at(tag).view;
    now.insert(tag);
    Size size{display.width, display.height};
    if (view.state) {
      auto state = std::static_pointer_cast<const ConcreteState<ModalHostViewState>>(view.state);
      if (state->getData().screenSize != size) state->updateState(ModalHostViewState(size));
    }
    if (shown_.count(tag)) continue;
    if (auto kind = animationOf(view); kind != ModalHostViewAnimationType::None) {
      entering_[tag] = Animation{kind == ModalHostViewAnimationType::Slide ? Kind::Slide : Kind::Fade, Clock::now(),
                                 testIdOf(tree_, tag)};
      Report::get().modalAnimation("in", toString(kind).c_str(), view, testIdOf(tree_, tag));
    }
    if (auto emitter = std::dynamic_pointer_cast<const ModalHostViewEventEmitter>(view.eventEmitter)) {
      emitter->onShow({});
      Report::get().deliver("show", 1, view, within(tree_, tag));
    }
  }
  for (auto it = entering_.begin(); it != entering_.end();) {
    it = now.count(it->first) ? std::next(it) : entering_.erase(it);
  }
  shown_ = std::move(now);
}

void ModalHost::leaving(const ShadowViewMutationList& mutations, const Display& display, const TextInputEdit& edit) {
  for (const auto& mutation : mutations) {
    if (mutation.type != ShadowViewMutation::Delete) continue;
    Tag tag = mutation.oldChildShadowView.tag;
    if (!shown_.count(tag) || !tree_.views().count(tag)) continue;
    const auto& view = tree_.views().at(tag).view;
    auto kind = animationOf(view);
    if (kind == ModalHostViewAnimationType::None) continue;
    auto frame = std::make_shared<Canvas>();
    drawModal(tree_, tag, display, *frame, edit);
    leaving_.push_back(Leaving{Animation{kind == ModalHostViewAnimationType::Slide ? Kind::Slide : Kind::Fade,
                                         Clock::now(), testIdOf(tree_, tag)},
                               std::move(frame)});
    Report::get().modalAnimation("out", toString(kind).c_str(), view, testIdOf(tree_, tag));
  }
}

bool ModalHost::tick(const Display& display) {
  // A frame is needed while one runs, and once more as the last ends, to draw the modal still.
  bool running = !entering_.empty() || !leaving_.empty();
  auto now = Clock::now();
  auto progress = [&](const Animation& animation) {
    return std::clamp(std::chrono::duration<float>(now - animation.start) / kAnimation, 0.0f, 1.0f);
  };
  for (auto it = entering_.begin(); it != entering_.end();) {
    float done = progress(it->second);
    auto presented = presentation(it->second, true, done, display);
    Report::get().modalPresented("in", it->second.name, presented.offset, presented.alpha,
                                   display.height);
    tree_.present(it->first, done < 1 ? std::optional(presented) : std::nullopt);
    it = done < 1 ? std::next(it) : entering_.erase(it);
  }
  auto& drawn = tree_.leaving();
  drawn.clear();
  for (const auto& leaving : leaving_) {
    auto presented = presentation(leaving.animation, false, progress(leaving.animation), display);
    Report::get().modalPresented("out", leaving.animation.name, presented.offset, presented.alpha,
                                   display.height);
    if (progress(leaving.animation) < 1) drawn.push_back({leaving.frame, presented});
  }
  std::erase_if(leaving_, [&](const Leaving& leaving) { return progress(leaving.animation) >= 1; });
  return running;
}

bool ModalHost::requestClose() {
  auto shown = modals(tree_);
  if (shown.empty()) return false;
  const auto& view = tree_.views().at(shown.back()).view;
  if (auto emitter = std::dynamic_pointer_cast<const ModalHostViewEventEmitter>(view.eventEmitter)) {
    emitter->onRequestClose({});
    Report::get().deliver("requestClose", 1, view, within(tree_, shown.back()));
  }
  return true;
}

}  // namespace oxyos
