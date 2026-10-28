// OxyOS platform TextLayoutManager: the interface React Native's platform text components use,
// implemented with the host text engine (TextLayoutManager.cpp, text.cpp). It takes the place of
// the `cxx` platform's header, which declares only `measure`; the text input's shadow node also
// measures lines for its baseline.
#pragma once

#include <react/renderer/attributedstring/AttributedStringBox.h>
#include <react/renderer/attributedstring/ParagraphAttributes.h>
#include <react/renderer/core/LayoutConstraints.h>
#include <react/renderer/textlayoutmanager/TextLayoutContext.h>
#include <react/renderer/textlayoutmanager/TextMeasureCache.h>
#include <react/utils/ContextContainer.h>

#include <cstdint>
#include <memory>

namespace facebook::react {

class TextLayoutManager {
 public:
  explicit TextLayoutManager(const std::shared_ptr<const ContextContainer>& contextContainer);
  virtual ~TextLayoutManager() = default;

  TextLayoutManager(const TextLayoutManager&) = delete;
  TextLayoutManager& operator=(const TextLayoutManager&) = delete;
  TextLayoutManager(TextLayoutManager&&) = delete;
  TextLayoutManager& operator=(TextLayoutManager&&) = delete;

  virtual TextMeasurement measure(
      const AttributedStringBox& attributedStringBox,
      const ParagraphAttributes& paragraphAttributes,
      const TextLayoutContext& layoutContext,
      const LayoutConstraints& layoutConstraints) const;

  // Measures text the platform cached by id. OxyOS caches no text outside the shadow tree, so a
  // text input's state never carries a cache id and this is never reached.
  TextMeasurement measureCachedSpannableById(
      int64_t cacheId,
      const ParagraphAttributes& paragraphAttributes,
      const TextLayoutContext& layoutContext,
      const LayoutConstraints& layoutConstraints) const;

  // Each laid-out line of the text within `size`.
  LinesMeasurements measureLines(
      const AttributedStringBox& attributedStringBox,
      const ParagraphAttributes& paragraphAttributes,
      const Size& size) const;

 protected:
  std::shared_ptr<const ContextContainer> contextContainer_;
  TextMeasureCache textMeasureCache_;
};

}  // namespace facebook::react
