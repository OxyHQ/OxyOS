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

class TextEditing {
 public:
  // `surfaceOf` names the surface a view is drawn on: text and keys reach the focused input only
  // when sent to that surface (typing into an open menu must not edit the field behind it).
  TextEditing(const ViewTree& tree, std::function<uint32_t(facebook::react::Tag)> surfaceOf)
      : tree_(tree), surfaceOf_(std::move(surfaceOf)) {}

  // A view command from JavaScript (`focus`, `blur`, `setTextAndSelection`).
  void command(const facebook::react::ShadowView& view, const std::string& name, const folly::dynamic& args);
  // Committed text and editing keys for the focused input.
  void handle(const OxyEvent& event);

  // The input that has focus, -1 if none: the renderer draws its caret.
  facebook::react::Tag focused() const { return focused_; }

 private:
  void focus(facebook::react::Tag tag);
  void blur();
  // Replaces the focused input's text as the user's edit: new state, then events.
  void edit(std::string text, const std::string& key);
  void submit();

  const ViewTree& tree_;
  std::function<uint32_t(facebook::react::Tag)> surfaceOf_;
  facebook::react::Tag focused_ = -1;
  std::string text_;
  int eventCount_ = 0;
};

}  // namespace oxy
