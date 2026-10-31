// The accessibility of what each surface shows, from React Native's accessibility props, as
// Android exposes a view hierarchy to TalkBack: the host publishes it to the compositor (the
// OxyOS display protocol's accessibility snapshots, ADR 0003), where an assistive technology reads
// it, and performs the assistive technology's actions. It also reports it (report.h).
#pragma once

#include "renderer.h"

#include <oxyos_client.h>

#include <folly/dynamic.h>

#include <functional>
#include <map>
#include <optional>
#include <string>
#include <utility>
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
  // disabled, selected, checked, expanded, busy, focused; live ("polite", "assertive")
  folly::dynamic state = folly::dynamic::object;
  facebook::react::Rect rect{};  // relative to the surface's root
  int depth = 0;       // nesting among nodes
  // What it offers beyond activate and focus (OXYOS_OFFERS_* bits), as Android's node info:
  // `accessibilityActions` increment and decrement, a scroll view that can still move each way,
  // the first node of a modal or a popup (dismiss), an editable text input (its selection).
  uint16_t actions = 0;
  // Its `accessibilityActions` other than these, by name, with their labels (the name without
  // one); "activate" among them makes activation that action rather than a click.
  std::vector<std::string> customNames, customLabels;
  bool activateAction = false;
  // A focused, editable, non-secure text input's selection, in bytes of its text.
  std::optional<std::pair<size_t, size_t>> selection;
};

// What the host does for an assistive technology's actions beyond activation and focus, and
// what it knows that a node shows (a scroll view's room, an input's selection).
struct AccessibilityHooks {
  // A scroll view's offset and furthest offset; nullopt for one that does not scroll.
  std::function<std::optional<std::pair<facebook::react::Point, facebook::react::Point>>(facebook::react::Tag)>
      scrollRange;
  // Scrolls a scroll view by (dx, dy); true if it moved.
  std::function<bool(facebook::react::Tag, float dx, float dy, uint32_t surface)> scroll;
  // Closes what the node on `surface` is the first node of: its modal or its popup.
  std::function<bool(uint32_t surface, facebook::react::Tag)> dismiss;
  // An input's selection, in bytes: set (taking focus), and read.
  std::function<bool(facebook::react::Tag, size_t start, size_t end)> select;
  std::function<std::optional<std::pair<size_t, size_t>>(facebook::react::Tag)> selection;
};

// The nodes under `root` in reading (tree) order, each parent before its children. An element
// is a view that is `accessible`, has a role or a label, or is a text input; text inside an
// accessible element is part of its name, not a node of its own. A scroll view that can move is
// an element too, named by its label alone, as Android's scrolling containers are nodes.
// `accessibilityElementsHidden` and `importantForAccessibility="no-hide-descendants"` hide a
// subtree. Popup views are left out: their content belongs to the popup's surface (`root` is
// then the popup view). The node with accessibility focus has "focused" in its state: the view
// `focus` if it is a node, else the first node inside it (as VoiceOver moves into a container).
std::vector<AccessibleNode> accessibilityTree(const ViewTree& tree, facebook::react::Tag root,
                                             facebook::react::Tag focus = -1,
                                             const AccessibilityHooks* hooks = nullptr);

folly::dynamic toDynamic(const std::vector<AccessibleNode>& nodes);

// Accessibility focus (Android's, apart from input focus), each surface's published snapshot,
// and the assistive technology's actions on it.
class AccessibilityBridge {
 public:
  AccessibilityBridge(const ViewTree& tree, Window& window) : tree_(tree), window_(window) {}

  // What performs actions beyond activation and focus, and reads scroll and selection state.
  void use(AccessibilityHooks hooks) { hooks_ = std::move(hooks); }

  // Moves accessibility focus (AccessibilityInfo.sendAccessibilityEvent(view, 'focus')).
  void focus(facebook::react::Tag tag) { focus_ = tag; }
  // The view with accessibility focus, or -1.
  facebook::react::Tag focused() const { return focus_; }

  // Publishes `surface`'s snapshot (its content under `root`, the root at `at` in the surface) if
  // it changed, and reports it.
  void publish(uint32_t surface, facebook::react::Tag root, facebook::react::Point at = {});
  // An assistive technology's action (OXYOS_EVENT_ACCESSIBILITY_ACTION), as TalkBack's are
  // performed on Android: activate is React Native's accessibility click (Pressability's
  // onPress), or its "activate" accessibility action where the node declares one; focus moves
  // accessibility focus; increment, decrement and custom actions are React Native's
  // accessibility actions (`onAccessibilityAction`, by name); scrolling moves a scroll view by
  // its viewport; dismiss closes the node's modal or popup; set-selection selects in an input.
  // Returns whether the focus changed (a new snapshot is due).
  bool act(const OxyOSEvent& event);
  // The surface went away (a popup closed).
  void forget(uint32_t surface) {
    published_.erase(surface);
    nodes_.erase(surface);
  }

 private:
  // What an action on a published node needs: its view, its custom actions' names, whether its
  // activation is its "activate" action.
  struct Published {
    facebook::react::Tag tag = -1;
    std::vector<std::string> customNames;
    bool activateAction = false;
  };
  const ViewTree& tree_;
  Window& window_;
  AccessibilityHooks hooks_;
  facebook::react::Tag focus_ = -1;
  // Per surface: the snapshot last published, and its nodes by index.
  std::map<uint32_t, folly::dynamic> published_;
  std::map<uint32_t, std::vector<Published>> nodes_;
};

}  // namespace oxyos
