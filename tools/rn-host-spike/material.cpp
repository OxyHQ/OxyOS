#include "material.h"

#include <cstring>

namespace facebook::react {

const char OxyOSMaterialViewComponentName[] = "OxyOSMaterialView";

}  // namespace facebook::react

namespace oxyos {

bool isMaterialView(const char* name) {
  return name && std::strcmp(name, facebook::react::OxyOSMaterialViewComponentName) == 0;
}

folly::dynamic materialReport(const std::vector<int32_t>& regions) {
  folly::dynamic list = folly::dynamic::array;
  for (size_t at = 0; at + 5 <= regions.size(); at += 5) {
    list.push_back(
        folly::dynamic::array(regions[at], regions[at + 1], regions[at + 2], regions[at + 3], regions[at + 4]));
  }
  return list;
}

}  // namespace oxyos
