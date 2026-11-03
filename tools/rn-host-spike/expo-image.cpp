#include "expo-image.h"

#include <react/renderer/core/propsConversions.h>

#include <algorithm>
#include <cctype>
#include <cmath>
#include <limits>

using namespace facebook::react;

namespace facebook::react {

const char ExpoImageComponentName[] = "ViewManagerAdapter_ExpoImage";

ExpoImageProps::ExpoImageProps(const PropsParserContext& context, const ExpoImageProps& sourceProps,
                               const RawProps& rawProps)
    : ViewProps(context, sourceProps, rawProps),
      source(convertRawProp(context, rawProps, "source", sourceProps.source, folly::dynamic(nullptr))),
      placeholder(convertRawProp(context, rawProps, "placeholder", sourceProps.placeholder, folly::dynamic(nullptr))),
      contentFit(convertRawProp(context, rawProps, "contentFit", sourceProps.contentFit, std::string("cover"))),
      placeholderContentFit(convertRawProp(context, rawProps, "placeholderContentFit",
                                           sourceProps.placeholderContentFit, std::string("scale-down"))),
      contentPosition(
          convertRawProp(context, rawProps, "contentPosition", sourceProps.contentPosition, folly::dynamic(nullptr))),
      transition(convertRawProp(context, rawProps, "transition", sourceProps.transition, folly::dynamic(nullptr))),
      tintColor(convertRawProp(context, rawProps, "tintColor", sourceProps.tintColor, SharedColor{})),
      blurRadius(convertRawProp(context, rawProps, "blurRadius", sourceProps.blurRadius, Float{0})),
      cachePolicy(convertRawProp(context, rawProps, "cachePolicy", sourceProps.cachePolicy, std::string("disk"))),
      priority(convertRawProp(context, rawProps, "priority", sourceProps.priority, std::string("normal"))),
      recyclingKey(convertRawProp(context, rawProps, "recyclingKey", sourceProps.recyclingKey, std::string())),
      autoplay(convertRawProp(context, rawProps, "autoplay", sourceProps.autoplay, true)),
      allowDownscaling(convertRawProp(context, rawProps, "allowDownscaling", sourceProps.allowDownscaling, true)) {}

}  // namespace facebook::react

namespace oxyos {

namespace {

double number(const folly::dynamic& value, double otherwise) {
  return value.isNumber() ? value.asDouble() : otherwise;
}

std::string lower(std::string text) {
  for (auto& c : text) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
  return text;
}

std::string percentDecoded(const std::string& text) {
  std::string out;
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

// One axis of ContentPosition from its start and end edges' values (left before right, as
// Android's offsetX tries them).
ExpoPositionAxis axis(const folly::dynamic& start, const folly::dynamic& end) {
  auto parsed = [](const folly::dynamic& value, bool reverse) -> std::optional<ExpoPositionAxis> {
    if (value.isNumber()) return ExpoPositionAxis{static_cast<float>(value.asDouble()), false, reverse};
    if (!value.isString()) return std::nullopt;
    std::string text = value.getString();
    if (text == "center") return ExpoPositionAxis{50, true, reverse};
    bool percentage = !text.empty() && text.back() == '%';
    if (percentage) text.pop_back();
    char* stop = nullptr;
    float amount = std::strtof(text.c_str(), &stop);
    if (stop == text.c_str()) return std::nullopt;
    // A number given as a string without `%` is a percentage too (Android removes the suffix and
    // takes what is left as one).
    return ExpoPositionAxis{amount, true, reverse};
  };
  if (auto at = parsed(start, false)) return *at;
  if (auto at = parsed(end, true)) return *at;
  return ExpoPositionAxis{};
}

// calcTranslation: a percentage of the room left (measured from the far edge when reversed), or a
// distance from the near or far edge.
float translation(const ExpoPositionAxis& axis, float image, float view) {
  if (axis.percentage) {
    float share = axis.reverse ? 100 - axis.value : axis.value;
    return share / 100 * (view - image);
  }
  return axis.reverse ? view - image - axis.value : axis.value;
}

}  // namespace

bool isExpoImage(const std::string& name) { return name == ExpoImageComponentName; }

bool isExpoImage(const ShadowView& view) { return view.componentName && isExpoImage(view.componentName); }

std::string ExpoImageSource::identity() const {
  std::string text = uri + "\n" + cacheKey + "\n" + std::to_string(width) + "x" + std::to_string(height) + "@" +
                     std::to_string(scale);
  for (const auto& [name, value] : headers) text += "\n" + name + ": " + value;
  return text;
}

std::vector<ExpoImageSource> expoImageSources(const folly::dynamic& value) {
  std::vector<ExpoImageSource> sources;
  if (!value.isArray()) return sources;
  for (const auto& entry : value) {
    if (!entry.isObject()) continue;
    ExpoImageSource source;
    if (auto uri = entry.get_ptr("uri"); uri && uri->isString()) source.uri = uri->getString();
    if (auto width = entry.get_ptr("width")) source.width = number(*width, 0);
    if (auto height = entry.get_ptr("height")) source.height = number(*height, 0);
    if (auto scale = entry.get_ptr("scale")) source.scale = number(*scale, 1);
    if (source.scale <= 0) source.scale = 1;
    if (auto key = entry.get_ptr("cacheKey"); key && key->isString()) source.cacheKey = key->getString();
    if (auto headers = entry.get_ptr("headers"); headers && headers->isObject()) {
      for (const auto& [name, header] : headers->items()) {
        if (name.isString() && header.isString()) source.headers.emplace_back(name.getString(), header.getString());
      }
      std::sort(source.headers.begin(), source.headers.end());
    }
    sources.push_back(std::move(source));
  }
  return sources;
}

std::optional<int64_t> expoImageRef(const folly::dynamic& value) {
  if (value.isNumber()) return static_cast<int64_t>(value.asDouble());
  return std::nullopt;
}

std::optional<ExpoImageSource> bestExpoImageSource(const std::vector<ExpoImageSource>& sources, double width,
                                                   double height) {
  if (sources.empty()) return std::nullopt;
  if (sources.size() == 1) return sources.front();
  double target = width * height;
  if (target <= 0) return std::nullopt;
  std::optional<ExpoImageSource> best;
  double bestFit = std::numeric_limits<double>::max();
  for (const auto& source : sources) {
    double fit = std::abs(1 - source.pixelCount() / target);
    if (fit < bestFit) {
      bestFit = fit;
      best = source;
    }
  }
  return best;
}

ExpoImageSourceKind expoImageSourceKind(const std::string& uri) {
  if (uri.empty()) return ExpoImageSourceKind::Unsupported;
  if (uri[0] == '/') return ExpoImageSourceKind::File;
  auto colon = uri.find(':');
  if (colon == std::string::npos) return ExpoImageSourceKind::Unsupported;
  std::string scheme = lower(uri.substr(0, colon));
  if (scheme == "http" || scheme == "https") return ExpoImageSourceKind::Remote;
  if (scheme == "file") return ExpoImageSourceKind::File;
  if (scheme == "data") return ExpoImageSourceKind::Data;
  if (scheme == "blurhash") return ExpoImageSourceKind::Blurhash;
  if (scheme == "thumbhash") return ExpoImageSourceKind::Thumbhash;
  return ExpoImageSourceKind::Unsupported;
}

std::string expoImageHash(const std::string& uri) {
  auto colon = uri.find(':');
  std::string path = colon == std::string::npos ? uri : uri.substr(colon + 1);
  while (!path.empty() && path.front() == '/') path.erase(path.begin());
  std::string hash = percentDecoded(path);
  if (lower(uri.substr(0, colon)) == "thumbhash") std::replace(hash.begin(), hash.end(), '\\', '/');
  return hash;
}

ExpoContentFit expoContentFit(const std::string& value, ExpoContentFit otherwise) {
  if (value == "contain") return ExpoContentFit::Contain;
  if (value == "cover") return ExpoContentFit::Cover;
  if (value == "fill") return ExpoContentFit::Fill;
  if (value == "none") return ExpoContentFit::None;
  if (value == "scale-down") return ExpoContentFit::ScaleDown;
  return otherwise;
}

ExpoContentPosition expoContentPosition(const folly::dynamic& value) {
  if (!value.isObject()) return ExpoContentPosition{};
  auto field = [&](const char* name) {
    auto found = value.get_ptr(name);
    return found ? *found : folly::dynamic(nullptr);
  };
  return ExpoContentPosition{axis(field("left"), field("right")), axis(field("top"), field("bottom"))};
}

ExpoPlacement placeExpoImage(ExpoContentFit fit, const ExpoContentPosition& position, float imageWidth,
                             float imageHeight, float boxWidth, float boxHeight, float sourceWidth,
                             float sourceHeight) {
  if (imageWidth <= 0 || imageHeight <= 0) return ExpoPlacement{};
  float sx = 1, sy = 1;
  auto within = [&](float targetWidth, float targetHeight) {
    sx = sy = std::min(targetWidth / imageWidth, targetHeight / imageHeight);
  };
  switch (fit) {
    case ExpoContentFit::Contain:
      within(boxWidth, boxHeight);
      break;
    case ExpoContentFit::Cover:
      sx = sy = std::max(boxWidth / imageWidth, boxHeight / imageHeight);
      break;
    case ExpoContentFit::Fill:
      sx = boxWidth / imageWidth, sy = boxHeight / imageHeight;
      break;
    case ExpoContentFit::None:
      break;
    case ExpoContentFit::ScaleDown:
      // As `contain` when the source is at least as large as the box on either axis, else its own
      // size (the source's, when it is known: the picture may be a smaller decode of it).
      if (sourceWidth > 0 && sourceHeight > 0) {
        if (sourceWidth >= boxWidth || sourceHeight >= boxHeight) {
          within(boxWidth, boxHeight);
        } else {
          within(sourceWidth, sourceHeight);
        }
      } else if (imageWidth >= boxWidth || imageHeight >= boxHeight) {
        within(boxWidth, boxHeight);
      }
      break;
  }
  ExpoPlacement placed{0, 0, imageWidth * sx, imageHeight * sy};
  placed.x = translation(position.x, placed.width, boxWidth);
  placed.y = translation(position.y, placed.height, boxHeight);
  return placed;
}

bool ExpoTransition::plays(const std::string& cacheType, bool initial) const {
  if (!initial) return true;
  switch (skip) {
    case Skip::None:
      return true;
    case Skip::Memory:
      return cacheType != "memory";
    case Skip::All:
      return cacheType != "memory" && cacheType != "disk";
  }
  return true;
}

std::optional<ExpoTransition> expoTransition(const folly::dynamic& value) {
  if (!value.isObject()) return std::nullopt;
  ExpoTransition transition;
  if (auto duration = value.get_ptr("duration")) transition.duration = static_cast<int>(number(*duration, 0));
  if (auto timing = value.get_ptr("timing"); timing && timing->isString()) {
    const auto& name = timing->getString();
    if (name == "ease-in") transition.timing = ExpoTransition::Timing::EaseIn;
    if (name == "ease-out") transition.timing = ExpoTransition::Timing::EaseOut;
    if (name == "linear") transition.timing = ExpoTransition::Timing::Linear;
  }
  if (auto skip = value.get_ptr("skipOnCacheHit"); skip && skip->isString()) {
    if (skip->getString() == "memory") transition.skip = ExpoTransition::Skip::Memory;
    if (skip->getString() == "all") transition.skip = ExpoTransition::Skip::All;
  }
  if (transition.duration <= 0) return std::nullopt;
  return transition;
}

float expoTransitionCurve(ExpoTransition::Timing timing, float t) {
  t = std::clamp(t, 0.0f, 1.0f);
  switch (timing) {
    case ExpoTransition::Timing::Linear:
      return t;
    case ExpoTransition::Timing::EaseIn:
      return t * t;
    case ExpoTransition::Timing::EaseOut:
      return 1 - (1 - t) * (1 - t);
    case ExpoTransition::Timing::EaseInOut:
      return static_cast<float>(std::cos((t + 1) * M_PI) / 2 + 0.5);
  }
  return t;
}

float expoBlurDeviation(float blurRadius) {
  // An Int prop on Android: a fraction is dropped.
  float radius = std::min(std::trunc(blurRadius), 25.0f);
  if (radius < 1) return 0;
  return 4 * std::sqrt(radius * (radius + 2) / 6);
}

}  // namespace oxyos
