// Text input on OxyOS: React Native's Android text input component (its props, state and shadow
// node, built unchanged) with an OxyOS descriptor, and the native editing side that Android's
// EditText provides: focus from JavaScript commands, committed text and editing keys from the
// compositor, state updates and events back to JavaScript.
#pragma once

#include "renderer.h"

#include <oxy_client.h>

#include <react/renderer/components/androidtextinput/AndroidTextInputShadowNode.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>
#include <react/renderer/textlayoutmanager/TextLayoutManager.h>

#include <folly/dynamic.h>

#include <functional>
#include <memory>
#include <optional>
#include <string>

namespace oxy {

// Android's descriptor without its JNI theme lookup: OxyOS has no platform text-field theme, so a
// text input takes only the padding its props give it (Bloom styles its own fields).
class TextInputComponentDescriptor final
    : public facebook::react::ConcreteComponentDescriptor<facebook::react::AndroidTextInputShadowNode> {
 public:
  explicit TextInputComponentDescriptor(const facebook::react::ComponentDescriptorParameters& parameters);

  facebook::react::State::Shared createInitialState(
      const facebook::react::Props::Shared& props,
      const facebook::react::ShadowNodeFamily::Shared& family) const override;

 protected:
  void adopt(facebook::react::ShadowNode& shadowNode) const override;

 private:
  std::shared_ptr<const facebook::react::TextLayoutManager> textLayoutManager_;
};

// Where a view is drawn: its surface and its frame in that surface's logical coordinates.
struct Placement {
  uint32_t surface = 0;
  facebook::react::Rect frame{};
};

// The editing a focused input receives, as Android's EditText does it: a caret and selection
// (keys, Shift, Ctrl+A, the pointer), committed text that replaces the selection, and an input
// method's composition (preedit), which is part of the text while it lasts.
class TextEditing {
 public:
  // `locate` places a view: text, keys and pointer input reach the focused input only on its
  // own surface (typing into an open menu must not edit the field behind it).
  TextEditing(const ViewTree& tree, std::function<std::optional<Placement>(facebook::react::Tag)> locate)
      : tree_(tree), locate_(std::move(locate)) {}

  // A view command from JavaScript (`focus`, `blur`, `setTextAndSelection`).
  void command(const facebook::react::ShadowView& view, const std::string& name, const folly::dynamic& args);
  // Compositor input: committed text, preedit, editing keys and pointer presses and drags.
  void handle(const OxyEvent& event);

  // Whether the focused input is secure (`secureTextEntry`): its text is never reported, only
  // as many bullets as it has characters (`logged`).
  bool secure() const;
  std::string logged(const std::string& text) const;

  // The input that has focus, -1 if none.
  facebook::react::Tag focused() const { return focused_; }
  // What the renderer draws for the focused input: its selection (or caret) and composition.
  TextInputEdit edit() const;

 private:
  void focus(facebook::react::Tag tag);
  void blur();
  void key(uint32_t code);
  void pointer(const OxyEvent& event);
  // The byte offset under a surface-local x, for the focused input.
  std::optional<size_t> offsetAt(uint32_t surface, float x, float y, bool inside) const;
  // Replaces bytes [from, to) with `insert` as the user's edit, the caret after it: new state,
  // then events, the key press (if any) first. `composing` keeps the inserted text as the
  // composition.
  void replace(size_t from, size_t to, const std::string& insert, const std::string& key, bool composing = false);
  // Dispatches onKeyPress for `key`, none if empty.
  void keyPress(const std::string& key);
  // Moves the selection; `extend` keeps the anchor.
  void select(size_t caret, bool extend);
  void selectionChanged();
  void submit();

  const ViewTree& tree_;
  std::function<std::optional<Placement>(facebook::react::Tag)> locate_;
  facebook::react::Tag focused_ = -1;
  std::string text_;
  size_t anchor_ = 0, caret_ = 0;  // byte offsets; the selection lies between them
  size_t composeStart_ = 0, composeEnd_ = 0;  // the composition, none if equal
  bool shift_ = false, ctrl_ = false;
  bool dragging_ = false;
  // The latest primary press: a press focuses its input on release (JavaScript sends `focus`),
  // and the caret goes where it was pressed.
  std::optional<std::pair<uint32_t, facebook::react::Point>> press_;
  int eventCount_ = 0;
};

}  // namespace oxy
