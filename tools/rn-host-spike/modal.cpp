#include "modal.h"

#include "report.h"

#include <react/renderer/components/FBReactNativeSpec/EventEmitters.h>
#include <react/renderer/components/modal/ModalHostViewState.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/core/ConcreteState.h>

using namespace facebook::react;

namespace oxyos {

namespace {

// The nearest view with a testID, the modal itself included, names it in the report.
folly::dynamic within(const ViewTree& tree, Tag tag) {
  auto path = ancestry(tree, tag);
  for (auto at = path.rbegin(); at != path.rend(); ++at) {
    auto props = std::dynamic_pointer_cast<const ViewProps>(tree.views().at(*at).view.props);
    if (props && !props->testId.empty()) return folly::dynamic::object("within", props->testId);
  }
  return folly::dynamic::object();
}

}  // namespace

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
    if (auto emitter = std::dynamic_pointer_cast<const ModalHostViewEventEmitter>(view.eventEmitter)) {
      emitter->onShow({});
      Report::get().deliver("show", 1, view, within(tree_, tag));
    }
  }
  shown_ = std::move(now);
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
