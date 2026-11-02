#include "android-view-props.h"

#include <memory>
#include <mutex>
#include <unordered_map>

using namespace facebook::react;

namespace oxyos {

namespace {

// Each View props' Android-only props, by the props they belong to, while they live; only those
// other than the defaults are kept.
struct Kept {
  std::mutex lock;
  std::unordered_map<const Props*, std::pair<std::weak_ptr<const Props>, AndroidViewProps>> byProps;
  size_t added = 0;
};

Kept& kept() {
  static Kept table;
  return table;
}

}  // namespace

AndroidViewProps androidViewPropsOf(const Props& props) {
  auto& table = kept();
  std::lock_guard<std::mutex> held(table.lock);
  auto it = table.byProps.find(&props);
  return it == table.byProps.end() ? AndroidViewProps{} : it->second.second;
}

Props::Shared ViewComponentDescriptor::cloneProps(const PropsParserContext& context, const Props::Shared& props,
                                                  RawProps rawProps) const {
  // The props as JavaScript gave them, before the portable props, which drop them, are made; one
  // set to null (removed) is back to its default.
  AndroidViewProps values = props ? androidViewPropsOf(*props) : AndroidViewProps{};
  if (!rawProps.isEmpty()) {
    folly::dynamic raw = rawProps.toDynamic();
    if (raw.isObject()) {
      if (auto it = raw.find("elevation"); it != raw.items().end()) {
        values.elevation = it->second.isNumber() ? static_cast<float>(it->second.asDouble()) : 0;
      }
      if (auto it = raw.find("needsOffscreenAlphaCompositing"); it != raw.items().end()) {
        values.needsOffscreenAlphaCompositing = it->second.isBool() && it->second.asBool();
      }
      if (auto it = raw.find("focusable"); it != raw.items().end()) {
        values.focusable = it->second.isBool() && it->second.asBool();
      }
      // A whole number of a map's, or -1.
      auto count = [](const folly::dynamic& map, const char* key) {
        if (!map.isObject()) return -1;
        auto at = map.find(key);
        return at != map.items().end() && at->second.isNumber() && at->second.asDouble() >= 0
                   ? static_cast<int>(at->second.asDouble())
                   : -1;
      };
      if (auto it = raw.find("accessibilityCollection"); it != raw.items().end()) {
        values.collectionRows = count(it->second, "rowCount");
        values.collectionColumns = count(it->second, "columnCount");
      }
      if (auto it = raw.find("accessibilityCollectionItem"); it != raw.items().end()) {
        values.itemRow = count(it->second, "rowIndex");
        values.itemColumn = count(it->second, "columnIndex");
      }
    }
  }
  auto made = facebook::react::ViewComponentDescriptor::cloneProps(context, props, std::move(rawProps));
  auto& table = kept();
  std::lock_guard<std::mutex> held(table.lock);
  // Props that died take their entries with them, now and then.
  if (++table.added % 256 == 0) {
    for (auto it = table.byProps.begin(); it != table.byProps.end();) {
      it = it->second.first.expired() ? table.byProps.erase(it) : std::next(it);
    }
  }
  if (values != AndroidViewProps{}) {
    table.byProps[made.get()] = {made, values};
  } else {
    table.byProps.erase(made.get());
  }
  return made;
}

}  // namespace oxyos
