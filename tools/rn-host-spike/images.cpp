#include "images.h"

#include "report.h"

#include <react/renderer/components/image/ImageEventEmitter.h>
#include <react/renderer/components/image/ImageProps.h>

#include <include/core/SkPixmap.h>

#include <cmath>
#include <iostream>
#include <stdexcept>

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
}

Images::~Images() { oxyos_images_free(loader_); }

void Images::sync(float scale) {
  // Views gone, or no longer images, show nothing more.
  for (auto it = shown_.begin(); it != shown_.end();) {
    auto view = tree_.views().find(it->first);
    bool image = view != tree_.views().end() && view->second.view.componentName &&
                 std::string(view->second.view.componentName) == "Image";
    it = image ? std::next(it) : shown_.erase(it);
  }
  for (const auto& [tag, entry] : tree_.views()) {
    if (!entry.view.componentName || std::string(entry.view.componentName) != "Image") continue;
    auto props = std::static_pointer_cast<const ImageProps>(entry.view.props);
    auto content = entry.view.layoutMetrics.getContentFrame().size;
    float width = std::ceil(content.width * scale), height = std::ceil(content.height * scale);
    auto source = chosen(props->sources, width, height);
    if (!source) {
      if (shown_.erase(tag)) tree_.setPicture(tag, nullptr);
      continue;
    }
    Shown shown{.uri = source->uri, .sourceScale = source->scale > 0 ? source->scale : 1};
    // Covering for cover, stretch and repeat; within for the others. Never larger than the file.
    bool covering = props->resizeMode == ImageResizeMode::Cover || props->resizeMode == ImageResizeMode::Stretch;
    auto resolved = resolveImageSource(source->uri);
    shown.key = Key{resolved.path, static_cast<uint32_t>(std::max(0.0f, width)),
                    static_cast<uint32_t>(std::max(0.0f, height)),
                    static_cast<uint32_t>(covering ? OXYOS_IMAGE_FIT_COVERING : OXYOS_IMAGE_FIT_WITHIN)};
    if (auto it = shown_.find(tag); it != shown_.end() && it->second.key == shown.key && it->second.uri == shown.uri) {
      continue;
    }
    if (!resolved.error.empty()) {
      shown_[tag] = shown;
      started(tag, shown);
      failed(tag, shown, resolved.error);
      continue;
    }
    // A picture another view already shows is the same picture.
    if (auto it = pictures_.find(shown.key); it != pictures_.end()) {
      if (auto picture = it->second.lock()) {
        shown_[tag] = shown;
        started(tag, shown);
        loaded(tag, shown, picture);
        continue;
      }
    }
    // A load of the same picture already on its way serves this view too.
    for (const auto& [id, load] : loads_) {
      if (load.first == shown.key) shown.loading = id;
    }
    if (!shown.loading) {
      shown.loading = ++next_;
      loads_[shown.loading] = {shown.key, shown.sourceScale};
      const auto& path = shown.key.path;
      if (oxyos_images_load(loader_, shown.loading, path.data(), path.size(), shown.key.width, shown.key.height,
                          shown.key.fit) != 0) {
        std::string error = oxyos_last_error();
        loads_.erase(shown.loading);
        shown.loading = 0;
        shown_[tag] = shown;
        started(tag, shown);
        failed(tag, shown, error);
        continue;
      }
    }
    shown_[tag] = shown;
    started(tag, shown);
  }
}

bool Images::deliver() {
  bool changed = false;
  OxyOSImageAnswer answer;
  while (oxyos_images_answer(loader_, &answer) == OXYOS_IMAGE_ANSWER) {
    auto load = loads_.find(answer.id);
    if (load == loads_.end()) continue;
    auto [key, sourceScale] = load->second;
    loads_.erase(load);
    std::shared_ptr<const Picture> picture;
    std::string error;
    if (answer.pixels) {
      auto made = std::make_shared<Picture>();
      made->width = static_cast<int>(answer.width);
      made->height = static_cast<int>(answer.height);
      // Premultiplied R, G, B, A bytes, rows top to bottom (oxyos_images_answer), copied into the
      // image Skia draws.
      made->image = SkImages::RasterFromPixmapCopy(
          SkPixmap(SkImageInfo::Make(made->width, made->height, kRGBA_8888_SkColorType, kPremul_SkAlphaType),
                   answer.pixels, static_cast<size_t>(made->width) * 4));
      made->naturalWidth = answer.natural_width / sourceScale;
      made->naturalHeight = answer.natural_height / sourceScale;
      picture = std::move(made);
      pictures_[key] = picture;
    } else {
      error.assign(answer.error, answer.error_length);
    }
    for (auto& [tag, shown] : shown_) {
      if (shown.loading != answer.id) continue;
      shown.loading = 0;
      if (picture) {
        loaded(tag, shown, picture);
      } else {
        failed(tag, shown, error);
      }
      changed = true;
    }
  }
  // Pictures no view shows any more are gone.
  std::erase_if(pictures_, [](const auto& entry) { return entry.second.expired(); });
  return changed;
}

void Images::started(Tag tag, const Shown&) {
  auto view = tree_.views().find(tag);
  if (view == tree_.views().end()) return;
  auto props = std::static_pointer_cast<const ImageProps>(view->second.view.props);
  if (!props->shouldNotifyLoadEvents) return;
  if (auto emitter = std::dynamic_pointer_cast<const ImageEventEmitter>(view->second.view.eventEmitter)) {
    emitter->onLoadStart();
    Report::get().deliver("loadStart", 0, view->second.view);
  }
}

void Images::loaded(Tag tag, const Shown& shown, const std::shared_ptr<const Picture>& picture) {
  tree_.setPicture(tag, picture);
  auto view = tree_.views().find(tag);
  if (view == tree_.views().end()) return;
  auto props = std::static_pointer_cast<const ImageProps>(view->second.view.props);
  if (!props->shouldNotifyLoadEvents) return;
  if (auto emitter = std::dynamic_pointer_cast<const ImageEventEmitter>(view->second.view.eventEmitter)) {
    // The bitmap's size in pixels, as Android reports it: size × scale.
    ImageSource source;
    source.uri = shown.uri;
    source.scale = shown.sourceScale;
    source.size = {picture->naturalWidth, picture->naturalHeight};
    emitter->onLoad(source);
    emitter->onLoadEnd();
    float width = picture->naturalWidth * shown.sourceScale, height = picture->naturalHeight * shown.sourceScale;
    Report::get().deliver("load", 0, view->second.view, folly::dynamic::object("width", width)("height", height));
    Report::get().deliver("loadEnd", 0, view->second.view);
  }
}

void Images::failed(Tag tag, const Shown& shown, const std::string& error) {
  // The view shows nothing, and the reason is told whether or not JavaScript listens.
  tree_.setPicture(tag, nullptr);
  if (reported_.insert(shown.uri).second) {
    std::cerr << "[host] image " << shown.uri << " not shown: " << error << "\n";
  }
  auto view = tree_.views().find(tag);
  if (view == tree_.views().end()) return;
  auto props = std::static_pointer_cast<const ImageProps>(view->second.view.props);
  if (!props->shouldNotifyLoadEvents) return;
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
