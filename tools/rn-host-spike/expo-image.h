// expo-image on OxyOS: its view (`ExpoImage`, the host component Expo names
// `ViewManagerAdapter_ExpoImage`) as expo-image's Android view (ExpoImageViewWrapper and
// ExpoImageView) behaves, on the host's one image pipeline (images.h: oxyos-image's loader, its
// memory and disk caches, the SVG decoder), never a loader of its own.
//
// What is here is the view's vocabulary: its props as Android's view definition takes them, the
// source chosen among several as Android's getBestSource chooses it, where a picture is drawn in
// the view's box (ContentFit's matrix moved by ContentPosition, ExpoImageView's
// applyTransformationMatrix), a transition's curve, and the blur Android's blurRadius gives.
// images.h loads what the view shows and holds what it draws (ImageLayers, renderer.h); the
// renderer draws it.
#pragma once

#include <react/renderer/components/view/ConcreteViewShadowNode.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>
#include <react/renderer/graphics/Color.h>
#include <react/renderer/mounting/ShadowView.h>

#include <folly/dynamic.h>

#include <cstdint>
#include <optional>
#include <string>
#include <utility>
#include <vector>

namespace facebook::react {

extern const char ExpoImageComponentName[];

// Android's ExpoImageModule view props (`Prop(...)`), each with its default there.
class ExpoImageProps final : public ViewProps {
 public:
  ExpoImageProps() = default;
  ExpoImageProps(const PropsParserContext& context, const ExpoImageProps& sourceProps, const RawProps& rawProps);

  // An array of sources ({uri, width, height, scale, headers, cacheKey}), or an ImageRef's number
  // (a shared object's id, expo-modules-core's processPropValue).
  folly::dynamic source;
  folly::dynamic placeholder;
  std::string contentFit{"cover"};
  std::string placeholderContentFit{"scale-down"};
  // {top, bottom, left, right}: each a number of logical pixels or a percentage string.
  folly::dynamic contentPosition;
  // {duration, timing, effect, skipOnCacheHit}, or null.
  folly::dynamic transition;
  SharedColor tintColor;
  Float blurRadius{0};
  std::string cachePolicy{"disk"};
  std::string priority{"normal"};
  std::string recyclingKey;
  bool autoplay{true};
  bool allowDownscaling{true};
};

using ExpoImageShadowNode = ConcreteViewShadowNode<ExpoImageComponentName, ExpoImageProps>;
using ExpoImageComponentDescriptor = ConcreteComponentDescriptor<ExpoImageShadowNode>;

}  // namespace facebook::react

namespace oxyos {

// Whether a component is expo-image's view.
bool isExpoImage(const std::string& name);
bool isExpoImage(const facebook::react::ShadowView& view);

// A source as Android's SourceMap takes it: its pixel count (width × height × scale²) is what
// getBestSource compares.
struct ExpoImageSource {
  std::string uri;
  double width = 0, height = 0, scale = 1;
  std::vector<std::pair<std::string, std::string>> headers;
  std::string cacheKey;
  double pixelCount() const { return width * height * scale * scale; }
  // What identifies the request it makes: a change of any of these loads it anew.
  std::string identity() const;
};

// The sources a `source` or `placeholder` prop gives (none for an ImageRef or nothing).
std::vector<ExpoImageSource> expoImageSources(const folly::dynamic& value);
// The ImageRef a `source` prop gives, by its shared object's id; none for sources.
std::optional<int64_t> expoImageRef(const folly::dynamic& value);
// Android's getBestSource for a view of `width` × `height` pixels: the one source there is,
// whatever the view's size; else the one whose pixel count is nearest the view's (|1 - its /
// the view's| least, the first of equals); none for none, or several in a view with no area.
std::optional<ExpoImageSource> bestExpoImageSource(const std::vector<ExpoImageSource>& sources, double width,
                                                   double height);

// The kinds of source Android's SourceMap tells apart.
enum class ExpoImageSourceKind { Remote, File, Data, Blurhash, Thumbhash, Unsupported };
ExpoImageSourceKind expoImageSourceKind(const std::string& uri);
// A `blurhash:/` or `thumbhash:/` URI's hash, as Android's model loaders read it: the path after
// the scheme, percent-decoded (expo-image's hashToUri encodes it), a ThumbHash's backslashes the
// slashes resolveThumbhashString replaced.
std::string expoImageHash(const std::string& uri);

// ContentFit (records' Android enum), CSS's object-fit.
enum class ExpoContentFit { Contain, Cover, Fill, None, ScaleDown };
ExpoContentFit expoContentFit(const std::string& value, ExpoContentFit otherwise);

// One axis of ContentPosition: a distance in logical pixels from its edge, or a percentage of the
// room left (`center` is 50%); `reverse` measures from the right or bottom edge.
struct ExpoPositionAxis {
  float value = 50;
  bool percentage = true;
  bool reverse = false;
};
struct ExpoContentPosition {
  ExpoPositionAxis x, y;
};
ExpoContentPosition expoContentPosition(const folly::dynamic& value);

// Where a picture of natural size `imageWidth` × `imageHeight` (logical pixels) is drawn in a box
// of `boxWidth` × `boxHeight`: ExpoImageView's matrix, ContentFit's scale (contain and scale-down
// as Matrix.ScaleToFit.START, then moved by the position; cover the larger scale; fill each axis
// its own; none its own size), then ContentPosition's translation of what it scaled. `sourceWidth`
// and `sourceHeight` (0: unknown) are what scale-down weighs, as Android's toMatrix weighs the
// source's size when it knows it.
struct ExpoPlacement {
  float x = 0, y = 0, width = 0, height = 0;
};
ExpoPlacement placeExpoImage(ExpoContentFit fit, const ExpoContentPosition& position, float imageWidth,
                             float imageHeight, float boxWidth, float boxHeight, float sourceWidth = 0,
                             float sourceHeight = 0);

// A transition (records' ImageTransition, with expo-image's `timing`): its duration in
// milliseconds, its curve, and the cache hits it is skipped on (`skipOnCacheHit`: none, memory,
// all). Every `effect` is a cross-dissolve, as on Android, whose ImageTransition has no effect.
struct ExpoTransition {
  int duration = 0;
  enum class Timing { EaseInOut, EaseIn, EaseOut, Linear } timing = Timing::EaseInOut;
  enum class Skip { None, Memory, All } skip = Skip::None;
  // Whether it plays for an image from `cacheType` ("none", "disk", "memory"), the view showing
  // nothing or a placeholder before (`initial`): ImageTransition.shouldPlay.
  bool plays(const std::string& cacheType, bool initial) const;
};
std::optional<ExpoTransition> expoTransition(const folly::dynamic& value);
// The share of a transition done at `t` (0 to 1) of its time: Android's interpolators,
// ViewPropertyAnimator's default AccelerateDecelerateInterpolator for ease-in-out (expo-image's
// default timing), AccelerateInterpolator for ease-in, DecelerateInterpolator for ease-out.
float expoTransitionCurve(ExpoTransition::Timing timing, float t);

// The Gaussian deviation, in the decoded picture's pixels, that Android's blurRadius gives: its
// SoftwareBlurTransformation stack-blurs the bitmap at a quarter of its size with the radius
// (at most 25), and a stack blur of radius r is a tent whose deviation is sqrt(r (r + 2) / 6).
// 0 for none.
float expoBlurDeviation(float blurRadius);

}  // namespace oxyos
