// OxyOS replacement for React Native's `cxx` TextLayoutManager stub: measures paragraphs with the
// host text engine, so Yoga sizes text exactly as the renderer will draw it.
#include <react/renderer/textlayoutmanager/TextLayoutManager.h>

#include "text.h"

#include <algorithm>

namespace facebook::react {

TextLayoutManager::TextLayoutManager(const std::shared_ptr<const ContextContainer>& contextContainer)
    : contextContainer_(contextContainer), textMeasureCache_(kSimpleThreadSafeCacheSizeCap) {}

TextMeasurement TextLayoutManager::measure(
    const AttributedStringBox& attributedStringBox,
    const ParagraphAttributes& paragraphAttributes,
    const TextLayoutContext& /*layoutContext*/,
    const LayoutConstraints& layoutConstraints) const {
  const auto& text = attributedStringBox.getValue();
  auto layout = oxy::TextEngine::shared().layout(
      text, paragraphAttributes, layoutConstraints.maximumSize.width);
  TextMeasurement::Attachments attachments;
  for (const auto& fragment : text.getFragments()) {
    if (fragment.isAttachment()) {
      attachments.push_back(TextMeasurement::Attachment{.frame = {}, .isClipped = true});
    }
  }
  return TextMeasurement{
      .size = layoutConstraints.clamp(Size{.width = std::ceil(layout.width), .height = std::ceil(layout.height)}),
      .attachments = attachments};
}

}  // namespace facebook::react
