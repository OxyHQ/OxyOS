// OxyOS replacement for React Native's `cxx` TextLayoutManager stub: measures paragraphs with the
// host text engine, so Yoga sizes text exactly as the renderer will draw it.
#include <react/renderer/textlayoutmanager/TextLayoutManager.h>

#include "text.h"

#include <react/debug/react_native_assert.h>

#include <glog/logging.h>

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
  auto layout = oxyos::TextEngine::shared().layout(
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

TextMeasurement TextLayoutManager::measureCachedSpannableById(
    int64_t cacheId,
    const ParagraphAttributes&,
    const TextLayoutContext&,
    const LayoutConstraints&) const {
  react_native_assert(false && "OxyOS issues no cached spannable ids");
  LOG(ERROR) << "measureCachedSpannableById(" << cacheId << ") on OxyOS, which caches no spannables";
  return {};
}

LinesMeasurements TextLayoutManager::measureLines(
    const AttributedStringBox& attributedStringBox,
    const ParagraphAttributes& paragraphAttributes,
    const Size& size) const {
  auto layout = oxyos::TextEngine::shared().layout(attributedStringBox.getValue(), paragraphAttributes, size.width);
  LinesMeasurements lines;
  for (const auto& line : layout.lines) {
    lines.emplace_back(line.text, Rect{{line.x, line.top}, {line.width, line.height}}, line.descent, line.capHeight,
                       line.ascent, line.xHeight);
  }
  return lines;
}

}  // namespace facebook::react
