#include "accessibility.h"

#include "report.h"
#include "window.h"

#include <react/renderer/components/androidtextinput/AndroidTextInputProps.h>
#include <react/renderer/components/androidtextinput/AndroidTextInputState.h>
#include <react/renderer/components/text/ParagraphState.h>
#include <react/renderer/components/view/BaseViewEventEmitter.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/components/view/accessibilityPropsConversions.h>
#include <react/renderer/core/ConcreteState.h>

#include <algorithm>
#include <cmath>
#include <initializer_list>
#include <unordered_map>

using namespace facebook::react;

namespace oxyos {

namespace {

std::string componentName(const ShadowView& view) { return view.componentName ? view.componentName : ""; }

bool hidden(const ViewProps& props) {
  return props.accessibilityElementsHidden ||
         props.importantForAccessibility == ImportantForAccessibility::NoHideDescendants;
}

std::string paragraphText(const ShadowView& view) {
  if (componentName(view) != "Paragraph" || !view.state) return {};
  return static_cast<const ConcreteState<ParagraphState>&>(*view.state).getData().attributedString.getString();
}

std::string inputText(const ShadowView& view) {
  if (!view.state) return {};
  return static_cast<const ConcreteState<AndroidTextInputState>&>(*view.state)
      .getData()
      .attributedStringBox.getValue()
      .getString();
}

// The text of a subtree, in order, as the name of an element that has no label. A text input
// inside contributes its placeholder, never its text.
void collectText(const ViewTree& tree, Tag tag, std::string& out) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  const auto& view = it->second.view;
  if (ownSurface(componentName(view))) return;
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  if (props && hidden(*props)) return;
  auto text = componentName(view) == "AndroidTextInput"
                  ? std::static_pointer_cast<const AndroidTextInputProps>(view.props)->placeholder
                  : paragraphText(view);
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

// Accessibility focus sent to a view that is not an element lands on the first node inside it,
// as VoiceOver moves to a container's first element (the behaviour Bloom's focus moves rely on:
// `useAccessibilityFocus` targets a non-accessible container).
struct Walk {
  const ViewTree& tree;
  Rect origin;   // the root's frame: node rectangles are relative to it
  Tag focus = -1;
  const AccessibilityHooks* hooks = nullptr;
  bool pending = false;  // inside the focused view, before its first node
  bool dismissable = false;  // inside a modal or a popup, before its first node
  std::vector<AccessibleNode> out;
};

// React Native's accessibility actions (`accessibilityActions`) as Android's node info takes
// them: increment and decrement are the node's own, "activate" its activation; any other is a
// custom action, by its label (its name without one). iOS's magic tap and escape have no node
// action on Android and are left out.
void declaredActions(const ViewProps& props, AccessibleNode& node) {
  for (const auto& action : props.accessibilityActions) {
    if (action.name == "increment") {
      node.actions |= OXYOS_OFFERS_INCREMENT;
    } else if (action.name == "decrement") {
      node.actions |= OXYOS_OFFERS_DECREMENT;
    } else if (action.name == "activate") {
      node.activateAction = true;
    } else if (action.name != "magicTap" && action.name != "escape") {
      auto label = action.label.value_or(action.name);
      // One label per line in the snapshot.
      std::replace(label.begin(), label.end(), '\n', ' ');
      node.customNames.push_back(action.name);
      node.customLabels.push_back(label);
    }
  }
}

void walk(Walk& w, Tag tag, bool isRoot, bool merged, int parent, int depth) {
  auto it = w.tree.views().find(tag);
  if (it == w.tree.views().end()) return;
  const auto& view = it->second.view;
  auto name = componentName(view);
  if (ownSurface(name) && !isRoot) return;
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  if (props && hidden(*props)) return;
  bool focusedHere = tag == w.focus;
  if (focusedHere) w.pending = true;
  // A modal's or a popup's first node is where an assistive technology dismisses it.
  bool dismissableHere = isModal(name) || (isRoot && name == kPopupComponent);
  bool wasDismissable = w.dismissable;
  if (dismissableHere) w.dismissable = true;
  auto add = [&](AccessibleNode node) {
    if (w.pending) {
      node.state["focused"] = true;
      w.pending = false;
    }
    if (w.dismissable) {
      node.actions |= OXYOS_OFFERS_DISMISS;
      w.dismissable = false;
    }
    node.tag = tag;
    node.parent = parent;
    node.depth = depth;
    if (auto frame = windowFrame(w.tree, tag)) {
      node.rect = Rect{{frame->origin.x - w.origin.origin.x, frame->origin.y - w.origin.origin.y}, frame->size};
    }
    w.out.push_back(std::move(node));
    return static_cast<int>(w.out.size()) - 1;
  };

  bool textInput = name == "AndroidTextInput";
  std::string role;
  if (props) role = props->role != Role::None ? toString(props->role) : props->accessibilityRole;
  if (textInput && role.empty()) role = "textinput";
  // How far a scroll view can still move back and on, if it scrolls at all.
  uint16_t scrolls = 0;
  if (name == "ScrollView" && w.hooks && w.hooks->scrollRange) {
    if (auto range = w.hooks->scrollRange(tag)) {
      auto [at, end] = *range;
      if (at.x > 0 || at.y > 0) scrolls |= OXYOS_OFFERS_SCROLL_BACKWARD;
      if (at.x < end.x || at.y < end.y) scrolls |= OXYOS_OFFERS_SCROLL_FORWARD;
    }
  }
  bool element = props && (props->accessible || !role.empty() || !props->accessibilityLabel.empty() ||
                           props->importantForAccessibility == ImportantForAccessibility::Yes || scrolls);
  if (element) {
    if (scrolls && role.empty()) role = "scrollview";
    AccessibleNode node{.role = role, .name = props->accessibilityLabel, .id = props->testId,
                        .state = stateOf(*props)};
    if (props->accessibilityLiveRegion == AccessibilityLiveRegion::Polite) node.state["live"] = "polite";
    if (props->accessibilityLiveRegion == AccessibilityLiveRegion::Assertive) node.state["live"] = "assertive";
    declaredActions(*props, node);
    node.actions |= scrolls;
    if (textInput) {
      auto input = std::static_pointer_cast<const AndroidTextInputProps>(view.props);
      if (node.name.empty()) node.name = input->placeholder;
      // A secure field's text never leaves the application, nor its selection.
      if (!input->secureTextEntry) {
        node.value = inputText(view);
        if (input->editable) node.actions |= OXYOS_OFFERS_SET_SELECTION;
        if (w.hooks && w.hooks->selection) node.selection = w.hooks->selection(tag);
      }
    } else if (node.name.empty() && !scrolls) {
      // A scroll view is named by its label alone: its content is read as its own nodes.
      collectText(w.tree, tag, node.name);
    }
    int index = add(std::move(node));
    // An accessible element is read as one: its text is its name. Elements inside it are still
    // nodes, as Android's focusable children are.
    for (Tag child : it->second.children) {
      walk(w, child, false, props->accessible || merged, index, depth + 1);
    }
  } else if (!merged && name == "Paragraph") {
    auto text = paragraphText(view);
    if (!text.empty()) {
      AccessibleNode node{.role = "text", .name = text};
      // Loose text can be a live region (Android's `accessibilityLiveRegion` on a TextView).
      if (props && props->accessibilityLiveRegion == AccessibilityLiveRegion::Polite) node.state["live"] = "polite";
      if (props && props->accessibilityLiveRegion == AccessibilityLiveRegion::Assertive) {
        node.state["live"] = "assertive";
      }
      add(std::move(node));
    }
  } else {
    for (Tag child : it->second.children) walk(w, child, false, merged, parent, depth);
  }
  // A focused view with no node inside it gives focus to nothing; a modal with none offers
  // nothing to dismiss.
  if (focusedHere) w.pending = false;
  if (dismissableHere && w.dismissable) w.dismissable = wasDismissable;
}

// React Native's role names (`role`, `accessibilityRole`) as the protocol's roles. A role the
// protocol does not name is published as generic.
uint8_t protocolRole(const std::string& role) {
  static const std::unordered_map<std::string, uint8_t> roles = {
      {"button", OXYOS_ROLE_BUTTON},         {"togglebutton", OXYOS_ROLE_BUTTON},
      {"imagebutton", OXYOS_ROLE_BUTTON},    {"link", OXYOS_ROLE_LINK},
      {"text", OXYOS_ROLE_TEXT},             {"header", OXYOS_ROLE_HEADING},
      {"heading", OXYOS_ROLE_HEADING},       {"image", OXYOS_ROLE_IMAGE},
      {"img", OXYOS_ROLE_IMAGE},             {"textinput", OXYOS_ROLE_TEXT_INPUT},
      {"searchbox", OXYOS_ROLE_SEARCH},      {"search", OXYOS_ROLE_SEARCH},
      {"menu", OXYOS_ROLE_MENU},             {"menuitem", OXYOS_ROLE_MENU_ITEM},
      {"checkbox", OXYOS_ROLE_CHECK_BOX},    {"radio", OXYOS_ROLE_RADIO},
      {"switch", OXYOS_ROLE_SWITCH},         {"tab", OXYOS_ROLE_TAB},
      {"tablist", OXYOS_ROLE_TAB_LIST},      {"list", OXYOS_ROLE_LIST},
      {"listitem", OXYOS_ROLE_LIST_ITEM},    {"dialog", OXYOS_ROLE_DIALOG},
      {"alertdialog", OXYOS_ROLE_DIALOG},    {"alert", OXYOS_ROLE_ALERT},
      {"progressbar", OXYOS_ROLE_PROGRESS_BAR}, {"adjustable", OXYOS_ROLE_SLIDER},
      {"slider", OXYOS_ROLE_SLIDER},         {"toolbar", OXYOS_ROLE_TOOLBAR},
      {"menubar", OXYOS_ROLE_TOOLBAR},       {"scrollview", OXYOS_ROLE_SCROLL_VIEW},
      {"group", OXYOS_ROLE_GROUP},           {"radiogroup", OXYOS_ROLE_GROUP},
      {"combobox", OXYOS_ROLE_COMBO_BOX},    {"summary", OXYOS_ROLE_SUMMARY},
      {"grid", OXYOS_ROLE_GRID},
  };
  auto it = roles.find(role);
  return it == roles.end() ? OXYOS_ROLE_GENERIC : it->second;
}

uint16_t protocolState(const folly::dynamic& state) {
  uint16_t bits = 0;
  auto flag = [&](const char* key) { return state.count(key) && state[key].isBool() && state[key].asBool(); };
  if (flag("disabled")) bits |= OXYOS_STATE_DISABLED;
  if (flag("selected")) bits |= OXYOS_STATE_SELECTED;
  if (flag("busy")) bits |= OXYOS_STATE_BUSY;
  if (flag("focused")) bits |= OXYOS_STATE_FOCUSED;
  if (state.count("expanded")) bits |= state["expanded"].asBool() ? OXYOS_STATE_EXPANDED : OXYOS_STATE_COLLAPSED;
  if (state.count("checked")) {
    auto checked = state["checked"].asString();
    if (checked == "checked") bits |= OXYOS_STATE_CHECKED;
    if (checked == "mixed") bits |= OXYOS_STATE_MIXED;
  }
  if (state.count("live")) bits |= state["live"].asString() == "assertive" ? OXYOS_STATE_LIVE_ASSERTIVE : OXYOS_STATE_LIVE_POLITE;
  return bits;
}

// The actions a node offers, by name, for the host's report.
folly::dynamic offeredNames(uint16_t actions) {
  folly::dynamic names = folly::dynamic::array;
  for (auto [bit, name] : std::initializer_list<std::pair<uint16_t, const char*>>{
           {OXYOS_OFFERS_INCREMENT, "increment"},
           {OXYOS_OFFERS_DECREMENT, "decrement"},
           {OXYOS_OFFERS_SCROLL_FORWARD, "scroll-forward"},
           {OXYOS_OFFERS_SCROLL_BACKWARD, "scroll-backward"},
           {OXYOS_OFFERS_DISMISS, "dismiss"},
           {OXYOS_OFFERS_SET_SELECTION, "select"}}) {
    if (actions & bit) names.push_back(name);
  }
  return names;
}

}  // namespace

std::vector<AccessibleNode> accessibilityTree(const ViewTree& tree, Tag root, Tag focus,
                                             const AccessibilityHooks* hooks) {
  Walk w{.tree = tree, .origin = windowFrame(tree, root).value_or(Rect{}), .focus = focus, .hooks = hooks};
  walk(w, root, true, false, -1, 0);
  return std::move(w.out);
}

folly::dynamic toDynamic(const std::vector<AccessibleNode>& nodes) {
  folly::dynamic list = folly::dynamic::array;
  for (const auto& node : nodes) {
    folly::dynamic entry = folly::dynamic::object("role", node.role)("name", node.name)("depth", node.depth);
    if (!node.value.empty()) entry["value"] = node.value;
    if (!node.id.empty()) entry["id"] = node.id;
    if (!node.state.empty()) entry["state"] = node.state;
    if (node.actions) entry["actions"] = offeredNames(node.actions);
    if (!node.customLabels.empty()) {
      entry["custom"] = folly::dynamic::array(node.customLabels.begin(), node.customLabels.end());
    }
    if (node.selection) entry["selection"] = folly::dynamic::array(node.selection->first, node.selection->second);
    list.push_back(std::move(entry));
  }
  return list;
}

void AccessibilityBridge::publish(uint32_t surface, Tag root, Point at) {
  auto nodes = accessibilityTree(tree_, root, focus_, &hooks_);
  for (auto& node : nodes) {
    node.rect.origin.x += at.x;
    node.rect.origin.y += at.y;
  }
  folly::dynamic reported = toDynamic(nodes);
  // Geometry is part of what is published (an assistive technology explores by position).
  folly::dynamic compared = folly::dynamic::array(reported);
  for (const auto& node : nodes) {
    compared.push_back(folly::dynamic::array(node.rect.origin.x, node.rect.origin.y, node.rect.size.width,
                                             node.rect.size.height, node.parent));
  }
  auto it = published_.find(surface);
  if (it != published_.end() && it->second == compared) return;
  published_[surface] = compared;
  Report::get().accessibility(surface, reported);

  std::vector<OxyOSAccessibleNode> wire;
  std::vector<Published> published;
  // The custom actions' labels, one per line, alive until the snapshot is sent.
  std::vector<std::string> labels;
  wire.reserve(nodes.size());
  labels.reserve(nodes.size());
  for (const auto& node : nodes) {
    std::string lines;
    for (const auto& label : node.customLabels) lines += (lines.empty() ? "" : "\n") + label;
    labels.push_back(std::move(lines));
  }
  for (size_t i = 0; i < nodes.size(); ++i) {
    const auto& node = nodes[i];
    auto rounded = [](float v) { return static_cast<int32_t>(std::lround(v)); };
    wire.push_back(OxyOSAccessibleNode{
        .parent = node.parent < 0 ? OXYOS_NO_ACCESSIBLE_PARENT : static_cast<uint32_t>(node.parent),
        .role = protocolRole(node.role),
        .state = protocolState(node.state),
        .x = rounded(node.rect.origin.x),
        .y = rounded(node.rect.origin.y),
        .width = static_cast<uint32_t>(std::max(0L, std::lround(node.rect.size.width))),
        .height = static_cast<uint32_t>(std::max(0L, std::lround(node.rect.size.height))),
        .name = node.name.c_str(),
        .value = node.value.c_str(),
        .actions = node.actions,
        .custom_actions = labels[i].c_str(),
        .has_selection = static_cast<uint8_t>(node.selection.has_value()),
        .selection_start = static_cast<uint32_t>(node.selection ? node.selection->first : 0),
        .selection_end = static_cast<uint32_t>(node.selection ? node.selection->second : 0),
    });
    published.push_back(Published{.tag = node.tag, .customNames = node.customNames,
                                  .activateAction = node.activateAction});
  }
  window_.publishAccessibility(surface, wire);
  nodes_[surface] = std::move(published);
}

bool AccessibilityBridge::act(const OxyOSEvent& event) {
  auto it = nodes_.find(event.surface);
  if (it == nodes_.end() || event.code >= it->second.size()) return false;
  const Published& node = it->second[event.code];
  Tag tag = node.tag;
  auto view = tree_.views().find(tag);
  if (view == tree_.views().end()) return false;
  const auto& shadow = view->second.view;
  std::string within;
  auto path = ancestry(tree_, tag);
  for (auto at = path.rbegin(); at != path.rend() && within.empty(); ++at) {
    auto props = std::dynamic_pointer_cast<const ViewProps>(tree_.views().at(*at).view.props);
    if (props && !props->testId.empty()) within = props->testId;
  }
  // React Native's accessibility action by name (`onAccessibilityAction`), as Android's delegate
  // sends topAccessibilityAction.
  auto named = [&](const std::string& name) {
    auto emitter = std::dynamic_pointer_cast<const BaseViewEventEmitter>(shadow.eventEmitter);
    if (!emitter) return false;
    emitter->onAccessibilityAction(name);
    Report::get().deliver("accessibilityAction", event.surface, shadow,
                          folly::dynamic::object("actionName", name)("within", within));
    return false;
  };
  switch (event.detail) {
    case OXYOS_ACTION_ACTIVATE:
      if (node.activateAction) return named("activate");
      // React Native's accessibility click: no pointerType, the node itself as target, which
      // Pressability answers with onPress (as TalkBack's activation on Android).
      shadow.eventEmitter->dispatchEvent("click", folly::dynamic::object());
      Report::get().deliver("click", event.surface, shadow, folly::dynamic::object("within", within));
      return false;
    case OXYOS_ACTION_FOCUS:
      if (focus_ == tag) return false;
      focus_ = tag;
      return true;
    case OXYOS_ACTION_INCREMENT:
      return named("increment");
    case OXYOS_ACTION_DECREMENT:
      return named("decrement");
    case OXYOS_ACTION_CUSTOM:
      if (event.width >= node.customNames.size()) return false;
      return named(node.customNames[event.width]);
    case OXYOS_ACTION_SCROLL_FORWARD:
    case OXYOS_ACTION_SCROLL_BACKWARD: {
      // By its viewport, as Android's scroll views scroll forward and backward.
      if (!hooks_.scroll) return false;
      Size page = shadow.layoutMetrics.frame.size;
      bool horizontal = false;
      if (auto range = hooks_.scrollRange ? hooks_.scrollRange(tag) : std::nullopt) {
        horizontal = range->second.y == 0 && range->second.x > 0;
      }
      float sign = event.detail == OXYOS_ACTION_SCROLL_FORWARD ? 1 : -1;
      hooks_.scroll(tag, horizontal ? sign * page.width : 0, horizontal ? 0 : sign * page.height, event.surface);
      return false;
    }
    case OXYOS_ACTION_DISMISS:
      if (hooks_.dismiss) hooks_.dismiss(event.surface, tag);
      return false;
    case OXYOS_ACTION_SET_SELECTION:
      if (hooks_.select) {
        hooks_.select(tag, static_cast<size_t>(std::max(0, event.x)), static_cast<size_t>(std::max(0, event.y)));
      }
      return false;
    default:
      return false;
  }
}

}  // namespace oxyos
