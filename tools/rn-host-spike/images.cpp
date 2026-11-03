#include "images.h"

#include "report.h"

#include <react/renderer/components/image/ImageEventEmitter.h>
#include <react/renderer/components/image/ImageProps.h>
#include <react/renderer/components/rnsvg/EventEmitters.h>
#include <react/renderer/components/rnsvg/Props.h>

#include <include/core/SkPixmap.h>

#include <sys/epoll.h>
#include <unistd.h>

#include <cmath>
#include <iostream>
#include <stdexcept>
#include <utility>

using namespace facebook::react;

namespace oxyos {

namespace {

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

// The source to show for a box of `width` × `height` pixels: the smallest that covers it, or the
// largest, as React Native's multiple sources are chosen.
std::optional<ImageSource> chosen(const ImageSources& sources, float width, float height) {
  auto pixels = [](const ImageSource& s) { return s.size.width * s.size.height * s.scale * s.scale; };
  float needed = width * height;
  std::optional<ImageSource> best;
  for (const auto& source : sources) {
    if (source.uri.empty()) continue;
    if (!best) {
      best = source;
      continue;
    }
    bool covers = pixels(source) >= needed, bestCovers = pixels(*best) >= needed;
    if (covers ? !bestCovers || pixels(source) < pixels(*best) : !bestCovers && pixels(source) > pixels(*best)) {
      best = source;
    }
  }
  return best;
}

bool isImage(const ShadowView& view) {
  return view.componentName && std::string(view.componentName) == "Image";
}
// react-native-svg's `<Image href>`, drawn by svg.cpp with the picture loaded here.
bool isSvgImage(const ShadowView& view) {
  return view.componentName && std::string(view.componentName) == "RNSVGImage";
}

// A length of react-native-svg's: a number, or a string with a number or a percentage of `base`.
float svgLength(const folly::dynamic& value, float base) {
  if (value.isNumber()) return static_cast<float>(value.asDouble());
  if (!value.isString()) return 0;
  const std::string& text = value.getString();
  char* end = nullptr;
  float number = std::strtof(text.c_str(), &end);
  return std::string(end) == "%" ? number / 100 * base : number;
}

// An SVG image element's box in pixels: its width and height in user units, through the view box of
// the SVG view holding it (react-native-svg's align and meetOrSlice) and the display's scale. The
// transforms of the groups between them are not counted: an image inside a scaled group is
// loaded at its unscaled size.
std::pair<float, float> svgImageBox(const ViewTree& tree, const View& image, float scale) {
  const auto& props = static_cast<const RNSVGImageProps&>(*image.view.props);
  Tag at = image.parent;
  while (at != -1) {
    auto it = tree.views().find(at);
    if (it == tree.views().end()) break;
    std::string name = it->second.view.componentName ? it->second.view.componentName : "";
    if (name == "RNSVGSvgView" || name == "RNSVGSvgViewAndroid") {
      auto content = it->second.view.layoutMetrics.getContentFrame().size;
      auto fit = [&](const auto& svg) {
        float vbWidth = svg.vbWidth > 0 ? svg.vbWidth : content.width;
        float vbHeight = svg.vbHeight > 0 ? svg.vbHeight : content.height;
        float sx = content.width / std::max(vbWidth, 1e-3f), sy = content.height / std::max(vbHeight, 1e-3f);
        if (svg.vbWidth > 0 && svg.vbHeight > 0 && svg.align != "none" && svg.meetOrSlice != 2) {
          sx = sy = svg.meetOrSlice == 1 ? std::max(sx, sy) : std::min(sx, sy);
        }
        return std::pair{svgLength(props.width, vbWidth) * sx * scale, svgLength(props.height, vbHeight) * sy * scale};
      };
      return name == "RNSVGSvgView" ? fit(static_cast<const RNSVGSvgViewProps&>(*it->second.view.props))
                                    : fit(static_cast<const RNSVGSvgViewAndroidProps&>(*it->second.view.props));
    }
    at = it->second.parent;
  }
  return {0, 0};
}

}  // namespace

ResolvedSource resolveImageSource(const std::string& uri) {
  if (uri.rfind("file://", 0) == 0) return {percentDecoded(uri.substr(7)), ""};
  if (!uri.empty() && uri[0] == '/') return {uri, ""};
  if (uri.rfind("http://", 0) == 0 || uri.rfind("https://", 0) == 0) {
    return {"", "remote images are not supported on OxyOS yet"};
  }
  if (uri.rfind("data:", 0) == 0) return {"", "data: images are not supported on OxyOS yet"};
  return {"", "no file for the image source " + uri};
}

Images::Images(ViewTree& tree) : tree_(tree), loader_(oxyos_images_new()) {
  if (!loader_) throw std::runtime_error(std::string("the image loader cannot start: ") + oxyos_last_error());
  // One descriptor for the host to wait on, readable when either decoder has an answer.
  epoll_ = epoll_create1(EPOLL_CLOEXEC);
  for (int fd : {oxyos_images_fd(loader_), svg_.fd()}) {
    epoll_event event{.events = EPOLLIN, .data = {.fd = fd}};
    if (epoll_ < 0 || epoll_ctl(epoll_, EPOLL_CTL_ADD, fd, &event) != 0) {
      if (epoll_ >= 0) close(epoll_);
      oxyos_images_free(loader_);
      throw std::runtime_error("the image loader cannot wait on its decoders");
    }
  }
}

Images::~Images() {
  close(epoll_);
  oxyos_images_free(loader_);
}

void Images::sync(float scale) {
  // Views gone, or no longer images, show nothing more.
  for (auto it = shown_.begin(); it != shown_.end();) {
    auto view = tree_.views().find(it->first);
    bool image = view != tree_.views().end() && (isImage(view->second.view) || isSvgImage(view->second.view));
    it = image ? std::next(it) : shown_.erase(it);
  }
  for (const auto& [tag, entry] : tree_.views()) {
    bool svgImage = isSvgImage(entry.view);
    if (!isImage(entry.view) && !svgImage) continue;
    // Its box in pixels, as it is drawn on the scale's pixel grid; how it fills it (resizeMode, or
    // an SVG image's preserveAspectRatio: slice covers its box, meet fits within it, none
    // stretches it, covering it).
    ImageSources sources;
    ImageResizeMode resizeMode = ImageResizeMode::Cover;
    float width = 0, height = 0;
    if (svgImage) {
      const auto& props = static_cast<const RNSVGImageProps&>(*entry.view.props);
      sources = {props.src};
      std::tie(width, height) = svgImageBox(tree_, entry, scale);
      width = std::round(width), height = std::round(height);
      resizeMode = props.meetOrSlice == 0 && props.align != "none" ? ImageResizeMode::Contain : ImageResizeMode::Cover;
    } else {
      auto props = std::static_pointer_cast<const ImageProps>(entry.view.props);
      auto content = entry.view.layoutMetrics.getContentFrame().size;
      width = static_cast<float>(gridPixels(content.width, scale));
      height = static_cast<float>(gridPixels(content.height, scale));
      sources = props->sources;
      resizeMode = props->resizeMode;
    }
    auto source = chosen(sources, width, height);
    if (!source) {
      if (shown_.erase(tag)) tree_.setPicture(tag, nullptr);
      continue;
    }
    Shown shown{.uri = source->uri, .sourceScale = source->scale > 0 ? source->scale : 1};
    // Covering for cover, stretch and repeat; within for the others. Never larger than the file.
    bool covering = resizeMode == ImageResizeMode::Cover || resizeMode == ImageResizeMode::Stretch;
    auto resolved = resolveImageSource(source->uri);
    // How the renderer places it (drawImage): an SVG document is drawn for that, at its natural
    // size at this scale for center, repeat and none.
    auto placement = resizeMode == ImageResizeMode::None ? SvgRequest::Placement::Natural
                     : resizeMode == ImageResizeMode::Center || resizeMode == ImageResizeMode::Repeat
                         ? SvgRequest::Placement::NaturalWithin
                         : SvgRequest::Placement::Fitted;
    shown.key = Key{resolved.path,
                    static_cast<uint32_t>(std::max(0.0f, width)),
                    static_cast<uint32_t>(std::max(0.0f, height)),
                    static_cast<uint32_t>(covering ? OXYOS_IMAGE_FIT_COVERING : OXYOS_IMAGE_FIT_WITHIN),
                    placement,
                    scale};
    auto was = shown_.find(tag);
    if (was != shown_.end() && was->second.key == shown.key && was->second.uri == shown.uri) {
      continue;
    }
    // The same source at another size (the view laid out anew) is decoded again for it, and told
    // no more: its loadStart was sent, and its load once it came.
    const bool same = was != shown_.end() && was->second.uri == shown.uri && was->second.sourceScale == shown.sourceScale;
    if (same) shown.told = was->second.told;
    if (!resolved.error.empty()) {
      auto& entry = shown_[tag] = shown;
      if (!same) {
        started(tag);
        failed(tag, entry, resolved.error);
      }
      continue;
    }
    // A picture another view already shows is the same picture.
    if (auto it = pictures_.find(shown.key); it != pictures_.end()) {
      if (auto picture = it->second.lock()) {
        auto& entry = shown_[tag] = shown;
        if (!same) started(tag);
        loaded(tag, entry, picture);
        continue;
      }
    }
    // A load of the same picture already on its way serves this view too.
    for (const auto& [id, load] : loads_) {
      if (load.key == shown.key) shown.loading = id;
    }
    if (!shown.loading) {
      shown.loading = ++next_;
      loads_[shown.loading] = Load{shown.key, shown.sourceScale};
      const auto& path = shown.key.path;
      if (svgByName(path)) {
        svg_.load(shown.loading, path, shown.key.svg());
      } else if (oxyos_images_load(loader_, shown.loading, path.data(), path.size(), shown.key.width, shown.key.height,
                          shown.key.fit) != 0) {
        std::string error = oxyos_last_error();
        loads_.erase(shown.loading);
        shown.loading = 0;
        auto& entry = shown_[tag] = shown;
        if (!same) started(tag);
        failed(tag, entry, error);
        continue;
      }
    }
    shown_[tag] = shown;
    if (!same) started(tag);
  }
}

bool Images::deliver() {
  bool changed = false;
  OxyOSImageAnswer answer;
  while (oxyos_images_answer(loader_, &answer) == OXYOS_IMAGE_ANSWER) {
    auto load = loads_.find(answer.id);
    if (load == loads_.end()) continue;
    if (!answer.pixels) {
      // A file oxyos-image cannot read may be an SVG document by its content, as Coil sniffs it:
      // the SVG decoder looks, and answers with this error if it is not one.
      svg_.load(answer.id, load->second.key.path, load->second.key.svg(), true,
                std::string(answer.error, answer.error_length));
      continue;
    }
    auto made = std::make_shared<Picture>();
    made->width = static_cast<int>(answer.width);
    made->height = static_cast<int>(answer.height);
    // Premultiplied R, G, B, A bytes, rows top to bottom (oxyos_images_answer), copied into the
    // image Skia draws.
    made->image = SkImages::RasterFromPixmapCopy(
        SkPixmap(SkImageInfo::Make(made->width, made->height, kRGBA_8888_SkColorType, kPremul_SkAlphaType),
                 answer.pixels, static_cast<size_t>(made->width) * 4));
    made->naturalWidth = answer.natural_width / load->second.sourceScale;
    made->naturalHeight = answer.natural_height / load->second.sourceScale;
    answered(answer.id, std::move(made), "");
    changed = true;
  }
  while (auto svg = svg_.answer()) {
    if (!loads_.count(svg->id)) continue;
    std::shared_ptr<Picture> made;
    if (svg->image.image) {
      made = std::make_shared<Picture>();
      made->width = svg->image.image->width();
      made->height = svg->image.image->height();
      made->image = std::move(svg->image.image);
      // Its natural size is in logical pixels already (CSS pixels, the document's own units).
      made->naturalWidth = svg->image.naturalWidth;
      made->naturalHeight = svg->image.naturalHeight;
    }
    answered(svg->id, std::move(made), svg->image.error);
    changed = true;
  }
  // Pictures no view shows any more are gone.
  std::erase_if(pictures_, [](const auto& entry) { return entry.second.expired(); });
  return changed;
}

void Images::answered(uint64_t id, std::shared_ptr<const Picture> picture, const std::string& error) {
  auto load = loads_.find(id);
  if (load == loads_.end()) return;
  Key key = load->second.key;
  loads_.erase(load);
  if (picture) pictures_[key] = picture;
  for (auto& [tag, shown] : shown_) {
    if (shown.loading != id) continue;
    shown.loading = 0;
    if (picture) {
      loaded(tag, shown, picture);
    } else {
      failed(tag, shown, error);
    }
  }
}

// Whether JavaScript hears the view's load events: an Image's when it listens
// (shouldNotifyLoadEvents); an SVG image's onLoad always, as react-native-svg's Android ImageView
// sends it (its only load event: it has no loadStart, error or loadEnd).
bool Images::notifies(const ShadowView& view) const {
  if (isSvgImage(view)) return true;
  return std::static_pointer_cast<const ImageProps>(view.props)->shouldNotifyLoadEvents;
}

void Images::started(Tag tag) {
  auto view = tree_.views().find(tag);
  if (view == tree_.views().end() || !isImage(view->second.view) || !notifies(view->second.view)) return;
  if (auto emitter = std::dynamic_pointer_cast<const ImageEventEmitter>(view->second.view.eventEmitter)) {
    emitter->onLoadStart();
    Report::get().deliver("loadStart", 0, view->second.view);
  }
}

void Images::loaded(Tag tag, Shown& shown, const std::shared_ptr<const Picture>& picture) {
  tree_.setPicture(tag, picture);
  if (std::exchange(shown.told, true)) return;
  auto view = tree_.views().find(tag);
  if (view == tree_.views().end() || !notifies(view->second.view)) return;
  // The bitmap's size in pixels, as Android reports it: size × scale.
  float width = picture->naturalWidth * shown.sourceScale, height = picture->naturalHeight * shown.sourceScale;
  if (auto emitter = std::dynamic_pointer_cast<const ImageEventEmitter>(view->second.view.eventEmitter)) {
    ImageSource source;
    source.uri = shown.uri;
    source.scale = shown.sourceScale;
    source.size = {picture->naturalWidth, picture->naturalHeight};
    emitter->onLoad(source);
    emitter->onLoadEnd();
    Report::get().deliver("load", 0, view->second.view, folly::dynamic::object("width", width)("height", height));
    Report::get().deliver("loadEnd", 0, view->second.view);
  } else if (auto svg = std::dynamic_pointer_cast<const RNSVGImageEventEmitter>(view->second.view.eventEmitter)) {
    svg->onLoad({.source = {.width = width, .height = height, .uri = shown.uri}});
    Report::get().deliver("load", 0, view->second.view, folly::dynamic::object("width", width)("height", height));
  }
}

void Images::failed(Tag tag, Shown& shown, const std::string& error) {
  if (reported_.insert(shown.uri).second) {
    std::cerr << "[host] image " << shown.uri << " not shown: " << error << "\n";
  }
  // Its source shown, decoded again at another size: the view keeps the picture it has.
  if (std::exchange(shown.told, true)) return;
  // The view shows nothing, and the reason is told whether or not JavaScript listens.
  tree_.setPicture(tag, nullptr);
  auto view = tree_.views().find(tag);
  if (view == tree_.views().end() || !isImage(view->second.view) || !notifies(view->second.view)) return;
  if (auto emitter = std::dynamic_pointer_cast<const ImageEventEmitter>(view->second.view.eventEmitter)) {
    ImageErrorInfo info;
    info.error = error;
    emitter->onError(info);
    emitter->onLoadEnd();
    Report::get().deliver("error", 0, view->second.view, folly::dynamic::object("error", error));
    Report::get().deliver("loadEnd", 0, view->second.view);
  }
}

}  // namespace oxyos
