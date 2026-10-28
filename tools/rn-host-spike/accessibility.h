// The accessibility tree of what a surface shows, from React Native's accessibility props, as
// Android exposes a view hierarchy to TalkBack. OxyOS has no assistive-technology bridge yet:
// the host reports this tree (report.h) so the semantics Bloom declares are checked on OxyOS.
#pragma once

#include "renderer.h"

#include <folly/dynamic.h>

#include <string>
#include <vector>

namespace oxy {

struct AccessibleNode {
  std::string role;    // `role`, else `accessibilityRole`; "text" for loose text, "textinput"
  std::string name;    // `accessibilityLabel`, else the text the element contains
  std::string id;      // testID, empty if none
  folly::dynamic state = folly::dynamic::object;  // disabled, selected, checked, expanded, busy when set
  int depth = 0;       // nesting among reported nodes
};

// The nodes under `root` in reading (tree) order. An element is a view that is `accessible`,
// has a role or a label, or is a text input; text inside an accessible element is part of its
// name, not a node of its own. `accessibilityElementsHidden` and
// `importantForAccessibility="no-hide-descendants"` hide a subtree. Popup views are left out:
// their content belongs to the popup's surface (`root` is then the popup view).
std::vector<AccessibleNode> accessibilityTree(const ViewTree& tree, facebook::react::Tag root);

folly::dynamic toDynamic(const std::vector<AccessibleNode>& nodes);

}  // namespace oxy
