#include "overscroll.h"

#include <memory>
#include <mutex>
#include <unordered_map>

using namespace facebook::react;

namespace oxyos {

namespace {

// Each ScrollView props' mode, by the props it belongs to, while they live.
struct Modes {
  std::mutex lock;
  std::unordered_map<const Props*, std::pair<std::weak_ptr<const Props>, OverScrollMode>> byProps;
  size_t added = 0;
};

Modes& modes() {
  static Modes table;
  return table;
}

OverScrollMode parse(const std::string& value) {
  if (value == "always") return OverScrollMode::Always;
  if (value == "never") return OverScrollMode::Never;
  return OverScrollMode::IfContentScrolls;
}

}  // namespace

OverScrollMode overScrollModeOf(const Props& props) {
  auto& table = modes();
  std::lock_guard<std::mutex> held(table.lock);
  auto it = table.byProps.find(&props);
  return it == table.byProps.end() ? OverScrollMode::IfContentScrolls : it->second.second;
}

Props::Shared ScrollViewComponentDescriptor::cloneProps(const PropsParserContext& context,
                                                        const Props::Shared& props, RawProps rawProps) const {
  // The prop as JavaScript gave it, before the portable props, which drop it, are made.
  std::optional<OverScrollMode> given;
  if (!rawProps.isEmpty()) {
    folly::dynamic raw = rawProps.toDynamic();
    if (raw.isObject()) {
      if (auto it = raw.find("overScrollMode"); it != raw.items().end()) {
        given = it->second.isString() ? parse(it->second.asString()) : OverScrollMode::IfContentScrolls;
      }
    }
  }
  OverScrollMode mode = given.value_or(props ? overScrollModeOf(*props) : OverScrollMode::IfContentScrolls);
  auto made = facebook::react::ScrollViewComponentDescriptor::cloneProps(context, props, std::move(rawProps));
  auto& table = modes();
  std::lock_guard<std::mutex> held(table.lock);
  // Props that died take their entries with them, now and then.
  if (++table.added % 256 == 0) {
    for (auto it = table.byProps.begin(); it != table.byProps.end();) {
      it = it->second.first.expired() ? table.byProps.erase(it) : std::next(it);
    }
  }
  if (mode != OverScrollMode::IfContentScrolls) {
    table.byProps[made.get()] = {made, mode};
  } else {
    table.byProps.erase(made.get());
  }
  return made;
}

}  // namespace oxyos
