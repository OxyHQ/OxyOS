#include "masked-view.h"

namespace facebook::react {

const char RNCMaskedViewComponentName[] = "RNCMaskedView";

}  // namespace facebook::react

namespace oxyos {

bool isMaskedView(const std::string& name) { return name == facebook::react::RNCMaskedViewComponentName; }

}  // namespace oxyos
