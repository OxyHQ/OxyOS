#include "textinput.h"

#include <react/renderer/components/androidtextinput/AndroidTextInputProps.h>
#include <react/renderer/components/androidtextinput/AndroidTextInputState.h>
#include <react/renderer/core/ConcreteState.h>
#include <react/renderer/core/EventEmitter.h>

#include <cstdlib>
#include <cstring>
#include <iostream>

using namespace facebook::react;

namespace oxy {

namespace {

// Linux input codes of the editing keys; text itself arrives committed, never as key codes.
constexpr uint32_t kBackspace = 14;
constexpr uint32_t kEnter = 28;
constexpr uint32_t kKeypadEnter = 96;

using TextInputState = ConcreteState<AndroidTextInputState>;

// JavaScript selections count UTF-16 code units.
int utf16Length(const std::string& text) {
  int length = 0;
  for (size_t i = 0; i < text.size(); ++i) {
    auto byte = static_cast<unsigned char>(text[i]);
    if ((byte & 0xc0) != 0x80) length += byte >= 0xf0 ? 2 : 1;
  }
  return length;
}

// Removes the last character (a whole UTF-8 sequence).
void removeLastCharacter(std::string& text) {
  if (text.empty()) return;
  size_t end = text.size() - 1;
  while (end > 0 && (static_cast<unsigned char>(text[end]) & 0xc0) == 0x80) --end;
  text.erase(end);
}

folly::dynamic selection(const std::string& text) {
  int caret = utf16Length(text);
  return folly::dynamic::object("start", caret)("end", caret);
}

}  // namespace

TextInputComponentDescriptor::TextInputComponentDescriptor(const ComponentDescriptorParameters& parameters)
    : ConcreteComponentDescriptor<AndroidTextInputShadowNode>(parameters),
      textLayoutManager_(std::make_shared<TextLayoutManager>(contextContainer_)) {}

State::Shared TextInputComponentDescriptor::createInitialState(
    const Props::Shared&,
    const ShadowNodeFamily::Shared& family) const {
  return std::make_shared<AndroidTextInputShadowNode::ConcreteState>(
      std::make_shared<const AndroidTextInputState>(AndroidTextInputState({}, {}, {}, 0)), family);
}

void TextInputComponentDescriptor::adopt(ShadowNode& shadowNode) const {
  auto& textInput = static_cast<AndroidTextInputShadowNode&>(shadowNode);
  // The shadow node measures its text (or placeholder) with the host text engine.
  textInput.setTextLayoutManager(textLayoutManager_);
  textInput.dirtyLayout();
  ConcreteComponentDescriptor::adopt(shadowNode);
}

void TextEditing::command(const ShadowView& view, const std::string& name, const folly::dynamic& args) {
  if (std::string(view.componentName ? view.componentName : "") != "AndroidTextInput") return;
  if (name == "focus") {
    focus(view.tag);
  } else if (name == "blur") {
    if (view.tag == focused_) blur();
  } else if (name == "setTextAndSelection" && view.tag == focused_ && args.isArray() && args.size() >= 2) {
    // JavaScript sets a controlled value. As on Android, a value older than the latest edit
    // is ignored; a current one replaces the text without an echoing change event.
    if (args[0].isInt() && args[0].asInt() < eventCount_) return;
    if (!args[1].isString() || args[1].getString() == text_) return;
    text_ = args[1].getString();
    auto it = tree_.views().find(focused_);
    if (it == tree_.views().end()) return;
    const auto& state = std::static_pointer_cast<const TextInputState>(it->second.view.state);
    auto props = std::static_pointer_cast<const AndroidTextInputProps>(it->second.view.props);
    auto data = state->getData();
    AttributedString string;
    string.setBaseTextAttributes(props->getEffectiveTextAttributes(1));
    if (!text_.empty()) {
      string.appendFragment({.string = text_, .textAttributes = string.getBaseTextAttributes(), .parentShadowView = {}});
    }
    data.attributedStringBox = AttributedStringBox{string};
    state->updateState(std::move(data));
  }
}

void TextEditing::focus(Tag tag) {
  if (tag == focused_) return;
  if (focused_ >= 0) blur();
  auto it = tree_.views().find(tag);
  if (it == tree_.views().end() || !it->second.view.state) return;
  auto props = std::static_pointer_cast<const AndroidTextInputProps>(it->second.view.props);
  if (!props->editable) return;
  const auto& data = std::static_pointer_cast<const TextInputState>(it->second.view.state)->getData();
  focused_ = tag;
  text_ = data.attributedStringBox.getValue().getString();
  eventCount_ = static_cast<int>(data.mostRecentEventCount);
  it->second.view.eventEmitter->dispatchEvent("focus", folly::dynamic::object("target", tag));
  if (std::getenv("OXY_HOST_TRACE")) std::cerr << "[text] focus " << tag << " \"" << text_ << "\"\n";
}

void TextEditing::blur() {
  auto it = tree_.views().find(focused_);
  Tag tag = focused_;
  focused_ = -1;
  if (it == tree_.views().end()) return;
  const auto& emitter = it->second.view.eventEmitter;
  emitter->dispatchEvent("blur", folly::dynamic::object("target", tag));
  emitter->dispatchEvent("endEditing", folly::dynamic::object("target", tag)("text", text_));
  if (std::getenv("OXY_HOST_TRACE")) std::cerr << "[text] blur " << tag << "\n";
}

void TextEditing::handle(const OxyEvent& event) {
  if (focused_ < 0) return;
  if (event.kind == OXY_EVENT_TEXT) {
    std::string committed(event.text, strnlen(event.text, sizeof(event.text)));
    edit(text_ + committed, committed);
  } else if (event.kind == OXY_EVENT_KEY && event.flag) {
    auto it = tree_.views().find(focused_);
    if (it == tree_.views().end()) return;
    auto props = std::static_pointer_cast<const AndroidTextInputProps>(it->second.view.props);
    if (event.code == kBackspace) {
      std::string text = text_;
      removeLastCharacter(text);
      edit(text, "Backspace");
    } else if (event.code == kEnter || event.code == kKeypadEnter) {
      bool newline = props->multiline && props->submitBehavior != SubmitBehavior::Submit &&
                     props->submitBehavior != SubmitBehavior::BlurAndSubmit;
      if (newline) {
        edit(text_ + "\n", "Enter");
      } else {
        submit();
      }
    }
  }
}

void TextEditing::edit(std::string text, const std::string& key) {
  auto it = tree_.views().find(focused_);
  if (it == tree_.views().end()) return;
  const auto& view = it->second.view;
  auto props = std::static_pointer_cast<const AndroidTextInputProps>(view.props);
  if (!props->editable) return;
  if (props->maxLength > 0 && utf16Length(text) > props->maxLength && text.size() > text_.size()) return;
  view.eventEmitter->dispatchEvent("keyPress", folly::dynamic::object("key", key));
  if (text == text_) return;
  text_ = std::move(text);
  ++eventCount_;

  // The native text becomes the input's state, as Android's EditText reports it; the shadow
  // node reconciles it with the value JavaScript sends back.
  const auto& state = std::static_pointer_cast<const TextInputState>(view.state);
  auto data = state->getData();
  AttributedString string;
  string.setBaseTextAttributes(props->getEffectiveTextAttributes(1));
  if (!text_.empty()) {
    string.appendFragment({.string = text_, .textAttributes = string.getBaseTextAttributes(), .parentShadowView = {}});
  }
  data.attributedStringBox = AttributedStringBox{string};
  data.mostRecentEventCount = eventCount_;
  state->updateState(std::move(data));

  view.eventEmitter->dispatchEvent(
      "change",
      folly::dynamic::object("text", text_)("eventCount", eventCount_)("target", focused_)("selection",
                                                                                         selection(text_)));
  view.eventEmitter->dispatchEvent("selectionChange", folly::dynamic::object("selection", selection(text_)));
  if (std::getenv("OXY_HOST_TRACE")) std::cerr << "[text] " << focused_ << " \"" << text_ << "\"\n";
}

void TextEditing::submit() {
  auto it = tree_.views().find(focused_);
  if (it == tree_.views().end()) return;
  auto props = std::static_pointer_cast<const AndroidTextInputProps>(it->second.view.props);
  it->second.view.eventEmitter->dispatchEvent("submitEditing",
                                              folly::dynamic::object("target", focused_)("text", text_));
  // A single-line input blurs on submit unless asked to stay (submitBehavior="submit").
  if (props->submitBehavior != SubmitBehavior::Submit) blur();
}

}  // namespace oxy
