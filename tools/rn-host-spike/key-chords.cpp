#include "key-chords.h"

#include <string>

namespace oxyos {

namespace {

constexpr uint32_t kLeftCtrl = 29, kRightCtrl = 97, kLeftShift = 42, kRightShift = 54, kLeftAlt = 56,
                   kRightAlt = 100, kLeftMeta = 125, kRightMeta = 126;

// A Linux input code's W3C `key` (unshifted, on the US layout) and `code`; null for none.
struct Key {
  const char* key;
  const char* code;
};

Key keyOf(uint32_t code) {
  // The letter rows, as the codes run along them.
  static const char* rows[] = {"qwertyuiop", "asdfghjkl", "zxcvbnm"};
  static const uint32_t starts[] = {16, 30, 44};
  static std::string letters[26], codes[26];
  for (int row = 0; row < 3; ++row) {
    std::string keys = rows[row];
    if (code >= starts[row] && code < starts[row] + keys.size()) {
      char letter = keys[code - starts[row]];
      int at = letter - 'a';
      letters[at] = std::string(1, letter);
      codes[at] = std::string("Key") + static_cast<char>(letter - 'a' + 'A');
      return {letters[at].c_str(), codes[at].c_str()};
    }
  }
  static const char* digits[] = {"1", "2", "3", "4", "5", "6", "7", "8", "9", "0"};
  static const char* digitCodes[] = {"Digit1", "Digit2", "Digit3", "Digit4", "Digit5",
                                     "Digit6", "Digit7", "Digit8", "Digit9", "Digit0"};
  if (code >= 2 && code <= 11) return {digits[code - 2], digitCodes[code - 2]};
  static const char* functions[] = {"F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10"};
  if (code >= 59 && code <= 68) return {functions[code - 59], functions[code - 59]};
  switch (code) {
    case 1: return {"Escape", "Escape"};
    case 12: return {"-", "Minus"};
    case 13: return {"=", "Equal"};
    case 14: return {"Backspace", "Backspace"};
    case 15: return {"Tab", "Tab"};
    case 26: return {"[", "BracketLeft"};
    case 27: return {"]", "BracketRight"};
    case 28: return {"Enter", "Enter"};
    case 39: return {";", "Semicolon"};
    case 40: return {"'", "Quote"};
    case 41: return {"`", "Backquote"};
    case 43: return {"\\", "Backslash"};
    case 51: return {",", "Comma"};
    case 52: return {".", "Period"};
    case 53: return {"/", "Slash"};
    case 57: return {" ", "Space"};
    case 87: return {"F11", "F11"};
    case 88: return {"F12", "F12"};
    case 96: return {"Enter", "NumpadEnter"};
    case 102: return {"Home", "Home"};
    case 103: return {"ArrowUp", "ArrowUp"};
    case 104: return {"PageUp", "PageUp"};
    case 105: return {"ArrowLeft", "ArrowLeft"};
    case 106: return {"ArrowRight", "ArrowRight"};
    case 107: return {"End", "End"};
    case 108: return {"ArrowDown", "ArrowDown"};
    case 109: return {"PageDown", "PageDown"};
    case 110: return {"Insert", "Insert"};
    case 111: return {"Delete", "Delete"};
    default: return {nullptr, nullptr};
  }
}

// What Shift types on the US layout, for the keys that are not letters.
const char* shifted(const std::string& key) {
  static const std::pair<const char*, const char*> pairs[] = {
      {"1", "!"}, {"2", "@"}, {"3", "#"}, {"4", "$"}, {"5", "%"}, {"6", "^"},  {"7", "&"},
      {"8", "*"}, {"9", "("}, {"0", ")"}, {"-", "_"}, {"=", "+"}, {"[", "{"},  {"]", "}"},
      {";", ":"}, {"'", "\""}, {"`", "~"}, {"\\", "|"}, {",", "<"}, {".", ">"}, {"/", "?"}};
  for (const auto& [plain, shift] : pairs) {
    if (key == plain) return shift;
  }
  return nullptr;
}

}  // namespace

std::optional<folly::dynamic> KeyChords::press(const OxyOSEvent& event) {
  if (event.kind != OXYOS_EVENT_KEY) return std::nullopt;
  bool down = event.flag != 0;
  switch (event.code) {
    case kLeftCtrl: case kRightCtrl: ctrl_ = down; return std::nullopt;
    case kLeftShift: case kRightShift: shift_ = down; return std::nullopt;
    case kLeftAlt: case kRightAlt: alt_ = down; return std::nullopt;
    case kLeftMeta: case kRightMeta: meta_ = down; return std::nullopt;
    default: break;
  }
  if (!down) {
    held_.erase(event.code);
    return std::nullopt;
  }
  auto [key, code] = keyOf(event.code);
  if (!key) return std::nullopt;
  std::string name = key;
  if (shift_ && name.size() == 1) {
    if (name[0] >= 'a' && name[0] <= 'z') {
      name[0] = static_cast<char>(name[0] - 'a' + 'A');
    } else if (const char* other = shifted(name)) {
      name = other;
    }
  }
  bool repeat = !held_.insert(event.code).second;
  return folly::dynamic::object("key", name)("code", code)("altKey", alt_)("ctrlKey", ctrl_)("metaKey", meta_)(
      "shiftKey", shift_)("repeat", repeat);
}

bool KeyChords::swallow(const OxyOSEvent& event) {
  if (event.kind != OXYOS_EVENT_KEY || event.flag) return false;
  return taken_.erase(event.code) > 0;
}

}  // namespace oxyos
