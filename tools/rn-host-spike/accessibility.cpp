#include "accessibility.h"

#include <react/renderer/components/androidtextinput/AndroidTextInputProps.h>
#include <react/renderer/components/androidtextinput/AndroidTextInputState.h>
#include <react/renderer/components/text/ParagraphState.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/components/view/accessibilityPropsConversions.h>
#include <react/renderer/core/ConcreteState.h>

using namespace facebook::react;

namespace oxy {

namespace {

std::string componentName(const ShadowView& view) { return view.componentName ? view.componentName : ""; }

bool hidden(const ViewProps& props) {
  return props.accessibilityElementsHidden ||
         props.importantForAccessibility == ImportantForAccessibility::NoHideDescendants;
}

// The text a view shows: a paragraph's string, or a text input's value (its placeholder when
// empty, as Android reads an EditText's hint).
std::string ownText(const ShadowView& view) {
  auto name = componentName(view);
  if (name == "Paragraph" && view.state) {
    return static_cast<const ConcreteState<ParagraphState>&>(*view.state).getData().attributedString.getString();
  }
  if (name == "AndroidTextInput" && view.state) {
    auto value = static_cast<const ConcreteState<AndroidTextInputState>&>(*view.state)
                     .getData()
                     .attributedStringBox.getValue()
                     .getString();
    return value.empty() ? std::static_pointer_cast<const AndroidTextInputProps>(view.props)->placeholder : value;
  }
  return {};
}

// The text of a subtree, in order, as the name of an element that has no label.
void collectText(const ViewTree& tree, Tag tag, std::string& out) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  const auto& view = it->second.view;
  if (componentName(view) == kPopupComponent) return;
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  if (props && hidden(*props)) return;
  auto text = ownText(view);
  if (!text.empty()) {
    if (!out.empty()) out += ' ';
    out += text;
  }
  for (Tag child : it->second.children) collectText(tree, child, out);
}

folly::dynamic stateOf(const ViewProps& props) {
  folly::dynamic state = folly::dynamic::object;
  if (!props.accessibilityState) return state;
  const auto& s = *props.accessibilityState;
  if (s.disabled) state["disabled"] = true;
  if (s.selected) state["selected"] = true;
  if (s.busy) state["busy"] = true;
  if (s.expanded) state["expanded"] = *s.expanded;
  if (s.checked != AccessibilityState::None) state["checked"] = toString(s.checked);
  return state;
}

void walk(const ViewTree& tree, Tag tag, bool isRoot, bool merged, int depth, std::vector<AccessibleNode>& out) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  const auto& view = it->second.view;
  auto name = componentName(view);
  if (name == kPopupComponent && !isRoot) return;
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  if (props && hidden(*props)) return;

  bool textInput = name == "AndroidTextInput";
  std::string role;
  if (props) role = props->role != Role::None ? toString(props->role) : props->accessibilityRole;
  if (textInput && role.empty()) role = "textinput";
  bool element = props && (props->accessible || !role.empty() || !props->accessibilityLabel.empty() ||
                           props->importantForAccessibility == ImportantForAccessibility::Yes);
  if (element) {
    AccessibleNode node{.role = role, .name = props->accessibilityLabel, .id = props->testId,
                        .state = stateOf(*props), .depth = depth};
    if (node.name.empty()) collectText(tree, tag, node.name);
    out.push_back(std::move(node));
    // An accessible element is read as one: its text is its name. Elements inside it are still
    // reported, as Android's focusable children are.
    for (Tag child : it->second.children) walk(tree, child, false, props->accessible || merged, depth + 1, out);
    return;
  }
  if (!merged && name == "Paragraph") {
    auto text = ownText(view);
    if (!text.empty()) out.push_back(AccessibleNode{.role = "text", .name = text, .depth = depth});
    return;
  }
  for (Tag child : it->second.children) walk(tree, child, false, merged, depth, out);
}

}  // namespace

std::vector<AccessibleNode> accessibilityTree(const ViewTree& tree, Tag root) {
  std::vector<AccessibleNode> nodes;
  walk(tree, root, true, false, 0, nodes);
  return nodes;
}

folly::dynamic toDynamic(const std::vector<AccessibleNode>& nodes) {
  folly::dynamic list = folly::dynamic::array;
  for (const auto& node : nodes) {
    folly::dynamic entry = folly::dynamic::object("role", node.role)("name", node.name)("depth", node.depth);
    if (!node.id.empty()) entry["id"] = node.id;
    if (!node.state.empty()) entry["state"] = node.state;
    list.push_back(std::move(entry));
  }
  return list;
}

}  // namespace oxy
