#include "keyboard-focus.h"

#include "android-view-props.h"

#include <react/renderer/components/androidtextinput/AndroidTextInputProps.h>
#include <react/renderer/components/view/ViewProps.h>

#include <algorithm>

using namespace facebook::react;

namespace oxyos {

namespace {

std::string componentName(const ShadowView& view) { return view.componentName ? view.componentName : ""; }

// Out of the keyboard's reach with everything in it: hidden from assistive technologies, taking no
// input (`pointerEvents: none`), transparent, or drawn in a surface of its own.
bool unreachable(const ShadowView& view, bool isRoot) {
  auto name = componentName(view);
  if (!isRoot && (ownSurface(name) || isModal(name))) return true;
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  if (!props) return false;
  return props->accessibilityElementsHidden ||
         props->importantForAccessibility == ImportantForAccessibility::NoHideDescendants ||
         props->pointerEvents == PointerEventsMode::None || props->opacity <= 0.001f;
}

bool disabled(const ViewProps& props) { return props.accessibilityState && props.accessibilityState->disabled; }

bool focusableItself(const ShadowView& view) {
  if (componentName(view) == "AndroidTextInput") {
    auto input = std::dynamic_pointer_cast<const AndroidTextInputProps>(view.props);
    return input && input->editable;
  }
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  if (!props || disabled(*props)) return false;
  // A control the assistive technology does not reach (a backdrop that closes a panel when
  // pressed, `accessible={false}`) is no stop of the keyboard's either: a Mac's key view loop and
  // VoiceOver go through the same controls.
  if (!androidViewPropsOf(*view.props).focusable || !props->accessible) return false;
  const auto& size = view.layoutMetrics.frame.size;
  return size.width > 0 && size.height > 0;
}

void collect(const ViewTree& tree, Tag tag, bool isRoot, std::vector<Tag>& out) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  const auto& view = it->second.view;
  if (unreachable(view, isRoot)) return;
  if (!isRoot && focusableItself(view)) out.push_back(tag);
  for (Tag child : it->second.children) collect(tree, child, false, out);
}

void modalRoot(const ViewTree& tree, Tag tag, bool isRoot, Tag& found) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  const auto& view = it->second.view;
  if (unreachable(view, isRoot)) return;
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  // The last in drawing order is on top.
  if (!isRoot && props && props->accessibilityViewIsModal) found = tag;
  for (Tag child : it->second.children) modalRoot(tree, child, false, found);
}

}  // namespace

Tag navigationRoot(const ViewTree& tree, Tag root) {
  Tag found = root;
  modalRoot(tree, root, true, found);
  return found;
}

std::vector<Tag> focusables(const ViewTree& tree, Tag root) {
  std::vector<Tag> out;
  collect(tree, root, true, out);
  return out;
}

bool focusable(const ViewTree& tree, Tag tag) {
  auto path = ancestry(tree, tag);
  if (path.empty()) return false;
  // From the view up to the root of the surface it is drawn in: a shell region, a popup or a
  // modal is its surface's root, whatever is above it.
  for (size_t i = path.size(); i-- > 0;) {
    const auto& view = tree.views().at(path[i]).view;
    if (unreachable(view, true)) return false;
    auto name = componentName(view);
    if (i + 1 < path.size() && (ownSurface(name) || isModal(name))) break;
  }
  return focusableItself(tree.views().at(tag).view);
}

bool editableInput(const ViewTree& tree, Tag tag) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end() || componentName(it->second.view) != "AndroidTextInput") return false;
  auto input = std::dynamic_pointer_cast<const AndroidTextInputProps>(it->second.view.props);
  return input && input->editable;
}

Tag KeyboardFocus::next(Tag root, bool backwards) const {
  auto all = focusables(tree_, root);
  if (all.empty()) return -1;
  auto at = std::find(all.begin(), all.end(), focused_);
  if (at == all.end()) return backwards ? all.back() : all.front();
  auto index = static_cast<size_t>(at - all.begin());
  if (backwards) return all[(index + all.size() - 1) % all.size()];
  return all[(index + 1) % all.size()];
}

Tag KeyboardFocus::recall(Tag root) {
  auto it = before_.find(root);
  if (it == before_.end()) return -1;
  Tag tag = it->second;
  before_.erase(it);
  return tag;
}

void KeyboardFocus::prune() {
  if (focused_ >= 0 && !tree_.views().count(focused_)) {
    focused_ = -1;
    ring_ = false;
  }
  for (auto it = before_.begin(); it != before_.end();) {
    it = tree_.views().count(it->first) ? std::next(it) : before_.erase(it);
  }
}

}  // namespace oxyos
