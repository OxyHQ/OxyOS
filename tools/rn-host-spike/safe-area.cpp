#include "safe-area.h"

#include "report.h"

using namespace facebook::react;

namespace facebook::react {

const char RNCSafeAreaProviderComponentName[] = "RNCSafeAreaProvider";
const char RNCSafeAreaViewComponentName[] = "RNCSafeAreaView";

}  // namespace facebook::react

namespace oxyos {

void SafeAreas::sync(const ViewTree& tree) {
  std::map<Tag, Rect> now;
  for (const auto& [tag, entry] : tree.views()) {
    const auto& view = entry.view;
    if (!view.componentName || std::string(view.componentName) != RNCSafeAreaProviderComponentName) continue;
    auto frame = windowFrame(tree, tag);
    if (!frame) continue;
    now[tag] = *frame;
    if (auto told = told_.find(tag); told != told_.end() && told->second == *frame) continue;
    folly::dynamic payload = folly::dynamic::object(
        "insets", folly::dynamic::object("top", 0)("right", 0)("bottom", 0)("left", 0))(
        "frame", folly::dynamic::object("x", frame->origin.x)("y", frame->origin.y)("width", frame->size.width)(
                     "height", frame->size.height));
    view.eventEmitter->dispatchEvent("insetsChange", std::move(payload));
    Report::get().deliver("insetsChange", 0, view);
  }
  told_ = std::move(now);
}

}  // namespace oxyos
