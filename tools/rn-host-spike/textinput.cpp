#include "textinput.h"

#include "report.h"

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

// Linux input codes of the editing keys and modifiers; text itself arrives committed (or as
// preedit), never as key codes.
constexpr uint32_t kBackspace = 14;
constexpr uint32_t kEnter = 28;
constexpr uint32_t kKeypadEnter = 96;
constexpr uint32_t kA = 30;
constexpr uint32_t kLeftCtrl = 29, kRightCtrl = 97;
constexpr uint32_t kLeftShift = 42, kRightShift = 54;
constexpr uint32_t kHome = 102, kLeft = 105, kRight = 106, kEnd = 107, kDelete = 111;
constexpr uint32_t kPrimaryButton = 272;

using TextInputState = ConcreteState<AndroidTextInputState>;

bool continuation(const std::string& text, size_t i) {
  return i < text.size() && (static_cast<unsigned char>(text[i]) & 0xc0) == 0x80;
}

// The character boundaries before and after byte offset `i` (whole UTF-8 sequences).
size_t previousBoundary(const std::string& text, size_t i) {
  if (i == 0) return 0;
  do --i;
  while (i > 0 && continuation(text, i));
  return i;
}

size_t nextBoundary(const std::string& text, size_t i) {
  if (i >= text.size()) return text.size();
  do ++i;
  while (continuation(text, i));
  return i;
}

// JavaScript selections count UTF-16 code units.
int utf16Length(const std::string& text, size_t bytes = std::string::npos) {
  int length = 0;
  bytes = std::min(bytes, text.size());
  for (size_t i = 0; i < bytes; ++i) {
    auto byte = static_cast<unsigned char>(text[i]);
    if ((byte & 0xc0) != 0x80) length += byte >= 0xf0 ? 2 : 1;
  }
  return length;
}

size_t byteOffset(const std::string& text, int units) {
  size_t i = 0;
  while (i < text.size() && units > 0) {
    units -= static_cast<unsigned char>(text[i]) >= 0xf0 ? 2 : 1;
    i = nextBoundary(text, i);
  }
  return i;
}

AttributedString attributed(const AndroidTextInputProps& props, const std::string& text) {
  AttributedString string;
  string.setBaseTextAttributes(props.getEffectiveTextAttributes(1));
  if (!text.empty()) {
    string.appendFragment({.string = text, .textAttributes = string.getBaseTextAttributes(), .parentShadowView = {}});
  }
  return string;
}

bool tracing() {
  static const bool trace = std::getenv("OXY_HOST_TRACE") != nullptr;
  return trace;
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
    // JavaScript sets a controlled value and selection (-1 for none). As on Android, a value
    // older than the latest edit is ignored; a current one replaces the text without an echoing
    // change event, and the selection stays where it was, kept within the new text.
    if (args[0].isInt() && args[0].asInt() < eventCount_) return;
    auto it = tree_.views().find(focused_);
    if (it == tree_.views().end()) return;
    size_t anchor = anchor_, caret = caret_;
    if (args[1].isString() && args[1].getString() != text_) {
      text_ = args[1].getString();
      composeStart_ = composeEnd_ = 0;
      const auto& state = std::static_pointer_cast<const TextInputState>(it->second.view.state);
      auto props = std::static_pointer_cast<const AndroidTextInputProps>(it->second.view.props);
      auto data = state->getData();
      data.attributedStringBox = AttributedStringBox{attributed(*props, text_)};
      state->updateState(std::move(data));
      auto clamp = [&](size_t at) {
        at = std::min(at, text_.size());
        while (continuation(text_, at)) --at;
        return at;
      };
      anchor = clamp(anchor);
      caret = clamp(caret);
    }
    if (args.size() >= 4 && args[2].isInt() && args[3].isInt() && args[2].asInt() >= 0 && args[3].asInt() >= 0) {
      anchor = byteOffset(text_, static_cast<int>(args[2].asInt()));
      caret = byteOffset(text_, static_cast<int>(args[3].asInt()));
    }
    if (anchor != anchor_ || caret != caret_) {
      anchor_ = anchor;
      caret_ = caret;
      selectionChanged();
    }
  }
}

TextInputEdit TextEditing::edit() const {
  if (focused_ < 0) return {};
  return TextInputEdit{.tag = focused_,
                       .selectionStart = std::min(anchor_, caret_),
                       .selectionEnd = std::max(anchor_, caret_),
                       .composeStart = composeStart_,
                       .composeEnd = composeEnd_};
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
  composeStart_ = composeEnd_ = 0;
  // The caret goes where the input was pressed; focus from JavaScript alone puts it at the end,
  // or selects everything with `selectTextOnFocus`.
  anchor_ = caret_ = text_.size();
  if (props->selectTextOnFocus) {
    anchor_ = 0;
  } else if (press_) {
    if (auto at = offsetAt(press_->first, press_->second.x, press_->second.y, true)) anchor_ = caret_ = *at;
  }
  it->second.view.eventEmitter->dispatchEvent("focus", folly::dynamic::object("target", tag));
  Report::get().deliver("focus", 0, it->second.view);
  selectionChanged();
  if (tracing()) std::cerr << "[text] focus " << tag << " \"" << text_ << "\" caret " << caret_ << "\n";
}

void TextEditing::blur() {
  auto it = tree_.views().find(focused_);
  Tag tag = focused_;
  focused_ = -1;
  dragging_ = false;
  // An unfinished composition stays as text, as Android's finishComposingText leaves it.
  composeStart_ = composeEnd_ = 0;
  if (it == tree_.views().end()) return;
  const auto& emitter = it->second.view.eventEmitter;
  emitter->dispatchEvent("blur", folly::dynamic::object("target", tag));
  emitter->dispatchEvent("endEditing", folly::dynamic::object("target", tag)("text", text_));
  Report::get().deliver("blur", 0, it->second.view, folly::dynamic::object("text", text_));
  if (tracing()) std::cerr << "[text] blur " << tag << "\n";
}

void TextEditing::handle(const OxyEvent& event) {
  if (event.kind == OXY_EVENT_POINTER) return pointer(event);
  if (event.kind == OXY_EVENT_KEY && (event.code == kLeftShift || event.code == kRightShift)) shift_ = event.flag;
  if (event.kind == OXY_EVENT_KEY && (event.code == kLeftCtrl || event.code == kRightCtrl)) ctrl_ = event.flag;
  if (event.kind == OXY_EVENT_FOCUS && !event.flag) shift_ = ctrl_ = false;
  if (focused_ < 0) return;
  if (event.kind != OXY_EVENT_TEXT && event.kind != OXY_EVENT_KEY && event.kind != OXY_EVENT_PREEDIT) return;
  auto placement = locate_(focused_);
  if (!placement || event.surface != placement->surface) return;
  size_t from = std::min(anchor_, caret_), to = std::max(anchor_, caret_);
  // Committed text and a new composition replace the composition, or else the selection.
  if (composeStart_ != composeEnd_) {
    from = composeStart_;
    to = composeEnd_;
  }
  // Key presses follow Android's input connection (ReactEditTextInputConnectionWrapper): a commit
  // of one character (two UTF-16 units at most) is that key, and a longer one none; a
  // composition is the character before its caret, or Backspace when the caret went back.
  if (event.kind == OXY_EVENT_TEXT) {
    std::string committed(event.text, strnlen(event.text, sizeof(event.text)));
    replace(from, to, committed, utf16Length(committed) <= 2 ? committed : "");
  } else if (event.kind == OXY_EVENT_PREEDIT) {
    std::string composition(event.text, strnlen(event.text, sizeof(event.text)));
    size_t before = caret_;
    bool selected = anchor_ != caret_;
    replace(from, to, composition, "", true);
    // The composing cursor, a byte offset within the composition.
    size_t cursor = std::min<size_t>(event.detail, composition.size());
    while (continuation(composition, cursor)) --cursor;
    if (from + cursor != caret_) {
      anchor_ = caret_ = from + cursor;
      selectionChanged();
    }
    bool back = caret_ < before || caret_ == 0 || (selected && caret_ == before);
    keyPress(back ? "Backspace" : text_.substr(previousBoundary(text_, caret_), caret_ - previousBoundary(text_, caret_)));
  } else if (event.flag) {
    key(event.code);
  }
}

void TextEditing::key(uint32_t code) {
  auto it = tree_.views().find(focused_);
  if (it == tree_.views().end()) return;
  auto props = std::static_pointer_cast<const AndroidTextInputProps>(it->second.view.props);
  size_t from = std::min(anchor_, caret_), to = std::max(anchor_, caret_);
  bool selected = from != to;
  // Editing keys end a composition first, keeping its text.
  if (code != kLeftShift && code != kRightShift && code != kLeftCtrl && code != kRightCtrl) {
    composeStart_ = composeEnd_ = 0;
  }
  switch (code) {
    case kBackspace:
      return replace(selected ? from : previousBoundary(text_, caret_), selected ? to : caret_, "", "Backspace");
    case kDelete:
      // A forward delete is no key press on Android (its input connection reports only
      // Backspace, Enter and digits).
      return replace(selected ? from : caret_, selected ? to : nextBoundary(text_, caret_), "", "");
    case kLeft:
      // Without Shift, a selection collapses to its start.
      return select(shift_ ? previousBoundary(text_, caret_) : selected ? from : previousBoundary(text_, caret_),
                    shift_);
    case kRight:
      return select(shift_ ? nextBoundary(text_, caret_) : selected ? to : nextBoundary(text_, caret_), shift_);
    case kHome:
      return select(0, shift_);
    case kEnd:
      return select(text_.size(), shift_);
    case kA:
      if (ctrl_ && (anchor_ != 0 || caret_ != text_.size())) {
        anchor_ = 0;
        caret_ = text_.size();
        selectionChanged();
      }
      return;
    case kEnter:
    case kKeypadEnter:
      if (props->multiline && props->submitBehavior != SubmitBehavior::Submit &&
          props->submitBehavior != SubmitBehavior::BlurAndSubmit) {
        return replace(from, to, "\n", "Enter");
      }
      return submit();
    default:
      return;
  }
}

std::optional<size_t> TextEditing::offsetAt(uint32_t surface, float x, float y, bool inside) const {
  auto it = tree_.views().find(focused_);
  auto placement = locate_(focused_);
  if (it == tree_.views().end() || !placement || placement->surface != surface) return std::nullopt;
  const auto& frame = placement->frame;
  if (inside && (x < frame.origin.x || y < frame.origin.y || x >= frame.origin.x + frame.size.width ||
                 y >= frame.origin.y + frame.size.height)) {
    return std::nullopt;
  }
  float local = x - frame.origin.x - it->second.view.layoutMetrics.getContentFrame().origin.x;
  return textInputOffsetAt(it->second.view, text_, local);
}

void TextEditing::pointer(const OxyEvent& event) {
  float x = event.x / 256.0f, y = event.y / 256.0f;
  if (event.detail == OXY_POINTER_PRESS && event.code == kPrimaryButton) {
    press_ = {event.surface, Point{x, y}};
    // A press in the focused input moves its caret (Shift extends the selection) and starts a
    // drag; a press in an unfocused input waits for its focus.
    if (focused_ < 0) return;
    if (auto at = offsetAt(event.surface, x, y, true)) {
      composeStart_ = composeEnd_ = 0;
      select(*at, shift_);
      dragging_ = true;
    }
  } else if (event.detail == OXY_POINTER_MOTION && dragging_) {
    if (auto at = offsetAt(event.surface, x, y, false)) select(*at, true);
  } else if ((event.detail == OXY_POINTER_RELEASE && event.code == kPrimaryButton) ||
             event.detail == OXY_POINTER_CANCEL || event.detail == OXY_POINTER_LEAVE) {
    dragging_ = false;
  }
}

void TextEditing::select(size_t caret, bool extend) {
  caret = std::min(caret, text_.size());
  size_t anchor = extend ? anchor_ : caret;
  if (anchor == anchor_ && caret == caret_) return;
  anchor_ = anchor;
  caret_ = caret;
  selectionChanged();
}

void TextEditing::selectionChanged() {
  auto it = tree_.views().find(focused_);
  if (it == tree_.views().end()) return;
  int start = utf16Length(text_, std::min(anchor_, caret_));
  int end = utf16Length(text_, std::max(anchor_, caret_));
  folly::dynamic selection = folly::dynamic::object("start", start)("end", end);
  it->second.view.eventEmitter->dispatchEvent("selectionChange", folly::dynamic::object("selection", selection));
  Report::get().deliver("selectionChange", 0, it->second.view, selection);
}

void TextEditing::replace(size_t from, size_t to, const std::string& insert, const std::string& key, bool composing) {
  auto it = tree_.views().find(focused_);
  if (it == tree_.views().end()) return;
  const auto& view = it->second.view;
  auto props = std::static_pointer_cast<const AndroidTextInputProps>(view.props);
  if (!props->editable) return;
  std::string text = text_.substr(0, from) + insert + text_.substr(to);
  if (props->maxLength > 0 && utf16Length(text) > props->maxLength && text.size() > text_.size()) return;
  keyPress(key);
  composeStart_ = composing && !insert.empty() ? from : 0;
  composeEnd_ = composing && !insert.empty() ? from + insert.size() : 0;
  size_t anchor = from + insert.size();
  bool moved = anchor != anchor_ || anchor != caret_;
  anchor_ = caret_ = anchor;
  if (text != text_) {
    text_ = std::move(text);
    ++eventCount_;

    // The native text becomes the input's state, as Android's EditText reports it; the shadow
    // node reconciles it with the value JavaScript sends back.
    const auto& state = std::static_pointer_cast<const TextInputState>(view.state);
    auto data = state->getData();
    data.attributedStringBox = AttributedStringBox{attributed(*props, text_)};
    data.mostRecentEventCount = eventCount_;
    state->updateState(std::move(data));

    int caret = utf16Length(text_, caret_);
    view.eventEmitter->dispatchEvent(
        "change", folly::dynamic::object("text", text_)("eventCount", eventCount_)("target", focused_)(
                      "selection", folly::dynamic::object("start", caret)("end", caret)));
    Report::get().deliver("change", 0, view, folly::dynamic::object("text", text_)("eventCount", eventCount_));
    if (tracing()) std::cerr << "[text] " << focused_ << " \"" << text_ << "\" caret " << caret_ << "\n";
  }
  if (moved) selectionChanged();
}

void TextEditing::keyPress(const std::string& key) {
  auto it = tree_.views().find(focused_);
  if (key.empty() || it == tree_.views().end()) return;
  it->second.view.eventEmitter->dispatchEvent("keyPress", folly::dynamic::object("key", key));
  Report::get().deliver("keyPress", 0, it->second.view, folly::dynamic::object("key", key));
}

void TextEditing::submit() {
  auto it = tree_.views().find(focused_);
  if (it == tree_.views().end()) return;
  auto props = std::static_pointer_cast<const AndroidTextInputProps>(it->second.view.props);
  it->second.view.eventEmitter->dispatchEvent("submitEditing",
                                              folly::dynamic::object("target", focused_)("text", text_));
  Report::get().deliver("submitEditing", 0, it->second.view, folly::dynamic::object("text", text_));
  // A single-line input blurs on submit unless asked to stay (submitBehavior="submit").
  if (props->submitBehavior != SubmitBehavior::Submit) blur();
}

}  // namespace oxy
