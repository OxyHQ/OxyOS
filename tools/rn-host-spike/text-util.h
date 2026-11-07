// Small text helpers the host's modules share: a name compared without its case (a media type, a
// URI's scheme, a file's extension), and a URI's percent-encoding undone (a file:// URI's path).
#pragma once

#include <cctype>
#include <string>

namespace oxyos {

// `text` with its ASCII letters in lowercase.
inline std::string lowercase(std::string text) {
  for (auto& c : text) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
  return text;
}

// `text` with each %XX decoded to its byte; a % not followed by two hex digits is kept.
inline std::string percentDecoded(const std::string& text) {
  std::string out;
  out.reserve(text.size());
  for (size_t i = 0; i < text.size(); ++i) {
    if (text[i] == '%' && i + 2 < text.size() && std::isxdigit(static_cast<unsigned char>(text[i + 1])) &&
        std::isxdigit(static_cast<unsigned char>(text[i + 2]))) {
      out += static_cast<char>(std::stoi(text.substr(i + 1, 2), nullptr, 16));
      i += 2;
    } else {
      out += text[i];
    }
  }
  return out;
}

}  // namespace oxyos
