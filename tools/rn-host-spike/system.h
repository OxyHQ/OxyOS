// Installing applications on the Bloom host (docs/adr/0030-installing-applications.mdx,
// decisions 8, 9 and 16): the session's answers, sheets and capability results as they come in
// chunks, and what this launch may use.
#pragma once

#include <cstdint>
#include <cstdlib>
#include <map>
#include <optional>
#include <set>
#include <sstream>
#include <string>

#include <folly/dynamic.h>

namespace oxyos {

// Texts the compositor sends in chunks, by the message they belong to (a request's serial, a
// sheet's number): a chunk out of order drops what was received.
class ChunkTexts {
 public:
  std::optional<std::string> add(uint32_t key, uint32_t index, bool last, const char* chunk) {
    auto& [text, next] = texts_[key];
    if (index == 0) {
      text.clear();
      next = 0;
    } else if (index != next) {
      texts_.erase(key);
      return std::nullopt;
    }
    text += chunk;
    next = index + 1;
    if (!last) return std::nullopt;
    std::string whole = std::move(text);
    texts_.erase(key);
    return whole;
  }

 private:
  std::map<uint32_t, std::pair<std::string, uint32_t>> texts_;
};

// What this launch may use (decision 8), as the session told it at launch (`OXYOS_APP_ORIGIN`,
// `OXYOS_APP_CAPABILITIES`): an installed version only what its account granted, and what it is
// granted at first use while it runs; a system or development application is not narrowed. A
// decision revoked in Settings takes effect at the next launch. With them, its system grants
// (`OXYOS_APP_GRANTS`, the image's policy's: `store`, `updates`...), so that JavaScript offers what
// a grant opens only with it (the store's installer with `store`, decision 9); the compositor
// checks every request against the grant itself.
class LaunchCapabilities {
 public:
  static LaunchCapabilities fromEnvironment() {
    LaunchCapabilities launch;
    const char* origin = std::getenv("OXYOS_APP_ORIGIN");
    launch.origin_ = origin ? origin : "development";
    auto names = [](const char* list, std::set<std::string>& into) {
      if (!list) return;
      std::stringstream names(list);
      for (std::string name; std::getline(names, name, ',');) {
        if (!name.empty()) into.insert(name);
      }
    };
    names(std::getenv("OXYOS_APP_CAPABILITIES"), launch.granted_);
    names(std::getenv("OXYOS_APP_GRANTS"), launch.grants_);
    return launch;
  }

  bool narrowed() const { return origin_ == "installed"; }
  bool allows(const std::string& capability) const { return !narrowed() || granted_.count(capability) > 0; }
  void grant(const std::string& capability) { granted_.insert(capability); }

  folly::dynamic describe() const {
    folly::dynamic granted = folly::dynamic::array;
    for (const auto& name : granted_) granted.push_back(name);
    folly::dynamic grants = folly::dynamic::array;
    for (const auto& name : grants_) grants.push_back(name);
    return folly::dynamic::object("origin", origin_)("capabilities", granted)("narrowed", narrowed())(
        "grants", grants);
  }

 private:
  std::string origin_;
  std::set<std::string> granted_;
  std::set<std::string> grants_;
};

// A capability asked at first use, by its protocol number (RequestCapability); 0 for another.
inline uint8_t capabilityCode(const std::string& name) {
  static const std::map<std::string, uint8_t> codes = {
      {"notifications", 1}, {"files", 2}, {"camera", 3}, {"microphone", 4}, {"location", 5}};
  auto found = codes.find(name);
  return found == codes.end() ? 0 : found->second;
}

}  // namespace oxyos
