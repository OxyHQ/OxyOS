// The accessibility of what each surface shows, from React Native's accessibility props, as
// Android exposes a view hierarchy to TalkBack: the host publishes it to the compositor (the
// OxyOS display protocol's accessibility snapshots, ADR 0003), where an assistive technology reads
// it, and performs the assistive technology's actions. It also reports it (report.h).
#pragma once

#include "renderer.h"

#include <oxyos_client.h>

#include <folly/dynamic.h>

#include <map>
#include <string>
#include <vector>

namespace oxyos {

class Window;

struct AccessibleNode {
  facebook::react::Tag tag = -1;
  int parent = -1;     // index of the parent node, -1 for a root
  std::string role;    // `role`, else `accessibilityRole`; "text" for loose text, "textinput"
  std::string name;    // `accessibilityLabel`, else the text the element contains (a text
                       // input: its placeholder)
  std::string value;   // a text input's text; never a secure field's
  std::string id;      // testID, empty if none
  folly::dynamic state = folly::dynamic::object;  // disabled, selected, checked, expanded, busy, focused
  facebook::react::Rect rect{};  // relative to the surface's root
  int depth = 0;       // nesting among nodes
};

// The nodes under `root` in reading (tree) order, each parent before its children. An element
// is a view that is `accessible`, has a role or a label, or is a text input; text inside an
// accessible element is part of its name, not a node of its own. `accessibilityElementsHidden`
// and `importantForAccessibility="no-hide-descendants"` hide a subtree. Popup views are left
// out: their content belongs to the popup's surface (`root` is then the popup view).
// The node with accessibility focus has "focused" in its state: the view `focus` if it is a
// node, else the first node inside it (as VoiceOver moves into a container).
std::vector<AccessibleNode> accessibilityTree(const ViewTree& tree, facebook::react::Tag root,
                                             facebook::react::Tag focus = -1);

folly::dynamic toDynamic(const std::vector<AccessibleNode>& nodes);

// Accessibility focus (Android's, apart from input focus), each surface's published snapshot,
// and the assistive technology's actions on it.
class AccessibilityBridge {
 public:
  AccessibilityBridge(const ViewTree& tree, Window& window) : tree_(tree), window_(window) {}

  // Moves accessibility focus (AccessibilityInfo.sendAccessibilityEvent(view, 'focus')).
  void focus(facebook::react::Tag tag) { focus_ = tag; }

  // Publishes `surface`'s snapshot (its content under `root`) if it changed, and reports it.
  void publish(uint32_t surface, facebook::react::Tag root);
  // An assistive technology's action (OXYOS_EVENT_ACCESSIBILITY_ACTION): activate is what
  // TalkBack's activation does, React Native's accessibility click (Pressability's onPress);
  // focus moves accessibility focus. Returns whether the focus changed (a new snapshot is due).
  bool act(const OxyOSEvent& event);
  // The surface went away (a popup closed).
  void forget(uint32_t surface) {
    published_.erase(surface);
    nodes_.erase(surface);
  }

 private:
  const ViewTree& tree_;
  Window& window_;
  facebook::react::Tag focus_ = -1;
  // Per surface: the snapshot last published, and its nodes' views by index.
  std::map<uint32_t, folly::dynamic> published_;
  std::map<uint32_t, std::vector<facebook::react::Tag>> nodes_;
};

}  // namespace oxyos
