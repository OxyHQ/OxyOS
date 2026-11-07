#include "images.h"

#include "report.h"
#include "system.h"

#include <react/renderer/components/image/ImageEventEmitter.h>
#include <react/renderer/components/image/ImageProps.h>
#include <react/renderer/components/rnsvg/EventEmitters.h>
#include <react/renderer/components/rnsvg/Props.h>

#include <include/core/SkCanvas.h>
#include <include/core/SkColorFilter.h>
#include <include/core/SkPaint.h>
#include <include/core/SkPixmap.h>
#include <include/core/SkSurface.h>

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
  auto scheme = uri.substr(0, uri.find(':'));
  for (auto& c : scheme) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
  if (scheme == "http" || scheme == "https") return {uri, "", true};
  if (scheme == "data") return {uri, "", false, true};
  return {"", "no file for the image source " + uri};
}

namespace {

// A picture tinted as Android's Drawable.setTint tints a loaded ImageRef's (SRC_IN), each frame.
std::shared_ptr<const Picture> tinted(const std::shared_ptr<const Picture>& picture, uint32_t argb) {
  auto made = std::make_shared<Picture>(*picture);
  SkPaint paint;
  paint.setColorFilter(SkColorFilters::Blend(SkColor(argb), SkBlendMode::kSrcIn));
  auto tint = [&](const sk_sp<SkImage>& image) -> sk_sp<SkImage> {
    if (!image) return image;
    auto surface = SkSurfaces::Raster(SkImageInfo::MakeN32Premul(image->width(), image->height()));
    if (!surface) return image;
    surface->getCanvas()->drawImage(image, 0, 0, SkSamplingOptions(), &paint);
    return surface->makeImageSnapshot();
  };
  for (auto& frame : made->frames) frame = tint(frame);
  made->image = made->frames.empty() ? tint(made->image) : made->frames.front();
  return made;
}

// A remote image's cache policy, as its source's `cache` says.
uint32_t cachePolicy(ImageSource::CacheStategy cache) {
  switch (cache) {
    case ImageSource::CacheStategy::Reload:
      return OXYOS_IMAGE_CACHE_RELOAD;
    case ImageSource::CacheStategy::ForceCache:
      return OXYOS_IMAGE_CACHE_FORCE;
    case ImageSource::CacheStategy::OnlyIfCached:
      return OXYOS_IMAGE_CACHE_ONLY_IF_CACHED;
    case ImageSource::CacheStategy::Default:
    default:
      return OXYOS_IMAGE_CACHE_DEFAULT;
  }
}

// The disk cache of remote images, in the application's cache directory, at most this large
// (Fresco's main disk cache's default).
constexpr uint64_t kDiskCacheBytes = 40ull << 20;


}  // namespace

Images::Images(ViewTree& tree, const DisplayRefresh& refresh)
    : tree_(tree), refresh_(refresh), loader_(oxyos_images_new()) {
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

void Images::connect(OxyOSNet* net, const std::string& cacheDirectory) {
  std::string directory = cacheDirectory.empty() ? "" : cacheDirectory + "/images";
  if (oxyos_images_connect(loader_, net, directory.empty() ? nullptr : directory.data(), directory.size(),
                           kDiskCacheBytes) != 0) {
    std::cerr << "[host] remote images are kept in memory alone: " << oxyos_last_error() << "\n";
    if (oxyos_images_connect(loader_, net, nullptr, 0, 0) != 0) return;
  }
  connected_ = true;
}

bool Images::fetch(uint64_t id, const std::string& uri, const ImageSource& source, uint32_t want, const Key& key,
                   std::string& error) {
  // React Native's prefetchImage fills the disk cache alone (Fresco's prefetchToDiskCache).
  return fetchRemote(id, uri, source.headers, cachePolicy(source.cache), want != OXYOS_IMAGE_WANT_PREFETCH, true, want,
                     key, "", error);
}

bool Images::fetchRemote(uint64_t id, const std::string& uri,
                         const std::vector<std::pair<std::string, std::string>>& given, uint32_t cache, bool memory,
                         bool disk, uint32_t want, const Key& key, const std::string& cacheKey, std::string& error) {
  // An installed version reaches the Internet only if its account granted `network`
  // (docs/adr/0030-installing-applications.mdx, decision 8), images included.
  if (!LaunchCapabilities::current().allows("network")) {
    error = kNotGrantedTheNetwork;
    return false;
  }
  if (!connected_) {
    error = "remote images need the network, which this host has not";
    return false;
  }
  std::vector<OxyOSNetHeader> headers;
  for (const auto& [name, value] : given) {
    headers.push_back({name.data(), name.size(), value.data(), value.size()});
  }
  OxyOSImageFetch request{
      .url = uri.data(),
      .url_length = uri.size(),
      .headers = headers.data(),
      .header_count = headers.size(),
      .cache = cache,
      .memory = static_cast<uint8_t>(memory),
      .disk = static_cast<uint8_t>(disk),
      .want = want,
      .width = key.width,
      .height = key.height,
      .fit = key.fit,
      .key = cacheKey.empty() ? nullptr : cacheKey.data(),
      .key_length = cacheKey.size(),
      .animated = static_cast<uint8_t>(key.animated),
  };
  if (oxyos_images_fetch(loader_, id, &request) != 0) {
    error = oxyos_last_error();
    return false;
  }
  Report::get().image("fetch", uri);
  return true;
}

bool Images::waited(uint64_t id) const {
  for (const auto& [tag, shown] : shown_) {
    if (shown.loading == id) return true;
  }
  for (const auto& [tag, shown] : expo_) {
    if (shown.main.loading == id || shown.placeholder.loading == id || shown.resized.loading == id) return true;
  }
  return refLoads_.count(id) > 0;
}

void Images::release(uint64_t id) {
  if (!id || !loads_.count(id) || waited(id)) return;
  bool remote = loads_[id].remote;
  std::string uri = loads_[id].key.path;
  loads_.erase(id);
  oxyos_images_cancel(loader_, id);
  if (remote) Report::get().image("cancelled", uri);
}

void Images::sync(float scale) {
  scale_ = scale;
  // expo-image views gone stop what they wait for.
  for (auto it = expo_.begin(); it != expo_.end();) {
    auto view = tree_.views().find(it->first);
    if (view != tree_.views().end() && isExpoImage(view->second.view)) {
      ++it;
      continue;
    }
    auto shown = std::move(it->second);
    it = expo_.erase(it);
    for (auto* request : {&shown.main, &shown.placeholder, &shown.resized}) cancel(*request);
  }
  // Views gone, or no longer images, show nothing more.
  for (auto it = shown_.begin(); it != shown_.end();) {
    auto view = tree_.views().find(it->first);
    bool image = view != tree_.views().end() && (isImage(view->second.view) || isSvgImage(view->second.view));
    if (image) {
      ++it;
      continue;
    }
    // Unmounted while it loads: its load is cancelled.
    uint64_t loading = it->second.loading;
    it = shown_.erase(it);
    release(loading);
  }
  for (const auto& [tag, entry] : tree_.views()) {
    if (isExpoImage(entry.view)) {
      syncExpo(tag, entry, scale);
      continue;
    }
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
      if (auto was = shown_.find(tag); was != shown_.end()) {
        uint64_t loading = was->second.loading;
        shown_.erase(was);
        tree_.setPicture(tag, nullptr);
        release(loading);
      }
      continue;
    }
    Shown shown{.uri = source->uri, .sourceScale = source->scale > 0 ? source->scale : 1};
    // Covering for cover, stretch and repeat; within for the others. Never larger than the file.
    bool covering = resizeMode == ImageResizeMode::Cover || resizeMode == ImageResizeMode::Stretch;
    auto resolved = resolveImageSource(source->uri);
    shown.remote = resolved.remote;
    if (resolved.remote && !LaunchCapabilities::current().allows("network")) resolved.error = kNotGrantedTheNetwork;
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
    // What it loaded before, for another source or size, it no longer waits for (its source
    // changed: cancelled, as Android's view's request is).
    uint64_t before = was != shown_.end() ? was->second.loading : 0;
    if (was != shown_.end()) was->second.loading = 0;
    if (!resolved.error.empty()) {
      auto& entry = shown_[tag] = shown;
      release(before);
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
        release(before);
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
      loads_[shown.loading] = Load{shown.key, shown.sourceScale, shown.remote, !shown.remote && !resolved.data};
      const auto& path = shown.key.path;
      std::string error;
      bool begun = true;
      if (shown.remote) {
        begun = fetch(shown.loading, path, *source, OXYOS_IMAGE_WANT_PICTURE, shown.key, error);
      } else if (resolved.data) {
        OxyOSImageOpen open{.kind = OXYOS_IMAGE_OPEN_DATA, .text = path.data(), .text_length = path.size(),
                            .width = shown.key.width, .height = shown.key.height, .fit = shown.key.fit};
        if (oxyos_images_open(loader_, shown.loading, &open) != 0) {
          error = oxyos_last_error();
          begun = false;
        }
      } else if (svgByName(path)) {
        svg_.load(shown.loading, path, shown.key.svg());
      } else if (oxyos_images_load(loader_, shown.loading, path.data(), path.size(), shown.key.width,
                                   shown.key.height, shown.key.fit) != 0) {
        error = oxyos_last_error();
        begun = false;
      }
      if (!begun) {
        loads_.erase(shown.loading);
        shown.loading = 0;
        auto& entry = shown_[tag] = shown;
        release(before);
        if (!same) started(tag);
        failed(tag, entry, error);
        continue;
      }
    }
    shown_[tag] = shown;
    release(before);
    if (!same) started(tag);
  }
}

bool Images::deliver() {
  bool changed = false;
  {
    std::lock_guard<std::mutex> hold(released_->lock);
    for (int64_t id : released_->ids) refs_.erase(id);
    released_->ids.clear();
  }
  OxyOSImageAnswer answer;
  while (oxyos_images_answer(loader_, &answer) == OXYOS_IMAGE_ANSWER) {
    if (requests_.count(answer.id)) {
      requestAnswered(answer.id, answer);
      continue;
    }
    auto load = loads_.find(answer.id);
    if (load == loads_.end()) continue;
    std::string error = answer.error ? std::string(answer.error, answer.error_length) : "";
    if (answer.kind == OXYOS_IMAGE_PROGRESS) {
      progressed(answer.id, answer.loaded, answer.total);
      continue;
    }
    if (answer.kind == OXYOS_IMAGE_UNDECODED) {
      // A remote image or a data: URI oxyos-image does not decode: an SVG document by its type, or
      // as Coil sniffs it; the SVG decoder looks, and answers with this error if it is not one.
      std::string type(answer.content_type ? answer.content_type : "", answer.content_type_length);
      bool svg = type.find("image/svg") != std::string::npos;
      auto bytes = std::make_shared<const std::string>(reinterpret_cast<const char*>(answer.data), answer.data_length);
      load->second.from = answer.from;
      svg_.load(answer.id, SvgSource(load->second.key.path, bytes), load->second.key.svg(), !svg, error);
      continue;
    }
    if (answer.kind == OXYOS_IMAGE_FAILED && load->second.file) {
      // A file oxyos-image cannot read may be an SVG document by its content, as Coil sniffs it:
      // the SVG decoder looks, and answers with this error if it is not one.
      svg_.load(answer.id, load->second.key.path, load->second.key.svg(), true, error);
      continue;
    }
    if (answer.kind != OXYOS_IMAGE_PICTURE || !answer.pixels) {
      answered(answer.id, nullptr, error);
      changed = true;
      continue;
    }
    load->second.from = answer.from;
    answered(answer.id, picture(answer, load->second.sourceScale), "");
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

std::shared_ptr<Picture> Images::picture(const OxyOSImageAnswer& answer, float sourceScale) {
  auto made = std::make_shared<Picture>();
  made->width = static_cast<int>(answer.width);
  made->height = static_cast<int>(answer.height);
  // Premultiplied R, G, B, A bytes, rows top to bottom (oxyos_images_answer), copied into the
  // images Skia draws: one, or each frame of an animated one.
  auto info = SkImageInfo::Make(made->width, made->height, kRGBA_8888_SkColorType, kPremul_SkAlphaType);
  size_t row = static_cast<size_t>(made->width) * 4, frame = row * static_cast<size_t>(made->height);
  uint32_t frames = std::max<uint32_t>(1, answer.frames);
  if (frames > 1 && answer.delays && answer.length >= frame * frames) {
    for (uint32_t i = 0; i < frames; ++i) {
      made->frames.push_back(SkImages::RasterFromPixmapCopy(SkPixmap(info, answer.pixels + frame * i, row)));
      made->delays.push_back(answer.delays[i]);
    }
    made->loops = answer.loops;
    made->image = made->frames.front();
  } else {
    made->image = SkImages::RasterFromPixmapCopy(SkPixmap(info, answer.pixels, row));
  }
  made->naturalWidth = answer.natural_width / sourceScale;
  made->naturalHeight = answer.natural_height / sourceScale;
  return made;
}

std::string Images::cacheType(uint32_t from) {
  return from == OXYOS_IMAGE_FROM_MEMORY ? "memory" : from == OXYOS_IMAGE_FROM_DISK ? "disk" : "none";
}

void Images::progressed(uint64_t id, uint64_t loaded, uint64_t total) {
  // An expo-image view's source, once for each source too (onProgress, OkHttpProgressListener's).
  for (const auto& [tag, shown] : expo_) {
    if (shown.main.loading != id || shown.told || total == 0) continue;
    emit(tag, "progress",
                                    folly::dynamic::object("loaded", static_cast<int64_t>(loaded))(
                                        "total", static_cast<int64_t>(total)));
  }
  for (const auto& [tag, shown] : shown_) {
    // What arrives of its source, once: never again when the view is decoded at another size.
    if (shown.loading != id || shown.told) continue;
    auto view = tree_.views().find(tag);
    if (view == tree_.views().end() || !isImage(view->second.view) || !notifies(view->second.view)) continue;
    if (auto emitter = std::dynamic_pointer_cast<const ImageEventEmitter>(view->second.view.eventEmitter)) {
      double progress = total > 0 ? static_cast<double>(loaded) / static_cast<double>(total) : 0;
      emitter->onProgress(progress, static_cast<int64_t>(loaded), static_cast<int64_t>(total));
      Report::get().deliver("progress", 0, view->second.view,
                            folly::dynamic::object("loaded", static_cast<int64_t>(loaded))(
                                "total", static_cast<int64_t>(total)));
    }
  }
}

void Images::size(const std::string& uri, const std::vector<std::pair<std::string, std::string>>& headers,
                  Sized done) {
  uint64_t id = ++next_;
  requests_[id] = Request{.uri = uri, .sized = std::move(done)};
  ImageSource source;
  source.uri = uri;
  source.headers = headers;
  std::string error;
  bool begun = true;
  if (resolveImageSource(uri).data) {
    // A data: URI's size, from its picture decoded whole (an SVG document's from its own).
    OxyOSImageOpen open{.kind = OXYOS_IMAGE_OPEN_DATA, .text = uri.data(), .text_length = uri.size()};
    if (oxyos_images_open(loader_, id, &open) != 0) {
      error = oxyos_last_error();
      begun = false;
    }
  } else {
    begun = fetch(id, uri, source, OXYOS_IMAGE_WANT_SIZE, Key{}, error);
  }
  if (!begun) {
    auto request = std::move(requests_[id]);
    requests_.erase(id);
    request.sized(0, 0, error);
  }
}

void Images::prefetch(const std::string& uri, int64_t requestId, Fetched done) {
  uint64_t id = ++next_;
  requests_[id] = Request{.uri = uri, .fetched = std::move(done), .requestId = requestId};
  ImageSource source;
  source.uri = uri;
  std::string error;
  if (!fetch(id, uri, source, OXYOS_IMAGE_WANT_PREFETCH, Key{}, error)) {
    auto request = std::move(requests_[id]);
    requests_.erase(id);
    request.fetched(error);
  }
}

void Images::abort(int64_t requestId) {
  for (auto it = requests_.begin(); it != requests_.end(); ++it) {
    if (!it->second.fetched || it->second.requestId != requestId) continue;
    oxyos_images_cancel(loader_, it->first);
    Report::get().image("cancelled", it->second.uri);
    auto request = std::move(it->second);
    requests_.erase(it);
    // As Android's abortRequest: the prefetch's promise is rejected.
    request.fetched("the prefetch was aborted");
    return;
  }
}

std::optional<std::string> Images::cached(const std::string& uri) {
  if (!resolveImageSource(uri).remote) return std::nullopt;
  int held = oxyos_images_cached(loader_, uri.data(), uri.size());
  if (held < 0) return std::nullopt;
  if (held & OXYOS_IMAGE_IN_MEMORY) return "memory";
  if (held & OXYOS_IMAGE_ON_DISK) return "disk";
  return std::nullopt;
}

void Images::requestAnswered(uint64_t id, const OxyOSImageAnswer& answer) {
  if (answer.kind == OXYOS_IMAGE_PROGRESS) return;
  auto request = std::move(requests_[id]);
  requests_.erase(id);
  std::string error = answer.error ? std::string(answer.error, answer.error_length) : "";
  if (request.fetched) return request.fetched(answer.kind == OXYOS_IMAGE_FETCHED ? "" : error);
  if (answer.kind == OXYOS_IMAGE_SIZE || answer.kind == OXYOS_IMAGE_PICTURE) {
    return request.sized(answer.natural_width, answer.natural_height, "");
  }
  if (answer.kind != OXYOS_IMAGE_UNDECODED) return request.sized(0, 0, error);
  // An SVG document's natural size (svg-image.h), by its type or its content.
  std::string type(answer.content_type ? answer.content_type : "", answer.content_type_length);
  auto bytes = std::make_shared<const std::string>(reinterpret_cast<const char*>(answer.data), answer.data_length);
  if (type.find("image/svg") == std::string::npos && !svgBySniffing(bytes->substr(0, 1024))) {
    return request.sized(0, 0, error);
  }
  std::string svgError;
  auto size = svgImageSize(SvgSource(request.uri, bytes), svgError);
  if (!size) return request.sized(0, 0, svgError);
  request.sized(size->first, size->second, "");
}

void Images::answered(uint64_t id, std::shared_ptr<const Picture> picture, const std::string& error) {
  auto load = loads_.find(id);
  if (load == loads_.end()) return;
  Key key = load->second.key;
  std::string from = cacheType(load->second.from);
  bool memory = load->second.memory;
  loads_.erase(load);
  if (picture && memory) pictures_[key] = picture;
  for (auto& [tag, shown] : shown_) {
    if (shown.loading != id) continue;
    shown.loading = 0;
    if (picture) {
      loaded(tag, shown, picture);
    } else {
      failed(tag, shown, error);
    }
  }
  for (auto& [tag, shown] : expo_) {
    for (auto* request : {&shown.main, &shown.placeholder, &shown.resized}) {
      if (request->loading != id) continue;
      if (picture) {
        expoLoaded(tag, shown, *request, picture, from);
      } else {
        expoFailed(tag, shown, *request, error);
      }
    }
  }
  if (auto waiting = refLoads_.find(id); waiting != refLoads_.end()) {
    int64_t ref = waiting->second;
    refLoads_.erase(waiting);
    auto held = refs_.find(ref);
    // Released before it arrived: nothing is told.
    if (held == refs_.end()) return;
    auto done = std::move(held->second.done);
    if (!picture) {
      refs_.erase(held);
      return done(0, nullptr, error);
    }
    held->second.picture = held->second.tint ? tinted(picture, *held->second.tint) : picture;
    done(ref, held->second.picture, "");
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

// expo-image's views (expo-image.h), as ExpoImageViewWrapper behaves.

bool Images::start(uint64_t id, const ExpoImageSource& source, const Key& key, const std::string& cachePolicy,
                   std::string& error) {
  auto open = [&](uint32_t kind, const std::string& text, uint32_t width, uint32_t height) {
    OxyOSImageOpen request{.kind = kind, .text = text.data(), .text_length = text.size(), .width = width,
                           .height = height, .fit = key.fit, .animated = static_cast<uint8_t>(key.animated)};
    if (oxyos_images_open(loader_, id, &request) == 0) return true;
    error = oxyos_last_error();
    return false;
  };
  switch (expoImageSourceKind(source.uri)) {
    case ExpoImageSourceKind::Remote: {
      // Glide's caches keep no HTTP freshness: what they hold is used however old.
      bool memory = cachePolicy == "memory" || cachePolicy == "memory-disk";
      bool disk = cachePolicy == "disk" || cachePolicy == "memory-disk";
      return fetchRemote(id, source.uri, source.headers,
                         memory || disk ? OXYOS_IMAGE_CACHE_FORCE : OXYOS_IMAGE_CACHE_RELOAD, memory, disk,
                         OXYOS_IMAGE_WANT_PICTURE, key, source.cacheKey, error);
    }
    case ExpoImageSourceKind::File: {
      auto resolved = resolveImageSource(source.uri);
      if (svgByName(resolved.path)) {
        svg_.load(id, resolved.path, key.svg());
        return true;
      }
      return open(OXYOS_IMAGE_OPEN_FILE, resolved.path, key.width, key.height);
    }
    case ExpoImageSourceKind::Data:
      return open(OXYOS_IMAGE_OPEN_DATA, source.uri, key.width, key.height);
    case ExpoImageSourceKind::Blurhash:
      // Decoded at the source's size (16 × 16 unless the hash string says), as BlurHashFetcher.
      return open(OXYOS_IMAGE_OPEN_BLURHASH, expoImageHash(source.uri),
                  static_cast<uint32_t>(source.width > 0 ? source.width : 16),
                  static_cast<uint32_t>(source.height > 0 ? source.height : 16));
    case ExpoImageSourceKind::Thumbhash:
      return open(OXYOS_IMAGE_OPEN_THUMBHASH, expoImageHash(source.uri), 0, 0);
    case ExpoImageSourceKind::Unsupported:
      break;
  }
  error = "OxyOS cannot load the image source " + source.uri;
  return false;
}

namespace {

// What a view's source is decoded at: covering its box for cover and fill, within it for contain
// and scale-down (ContentFitDownsampleStrategy: never enlarged), whole for none or without
// downscaling; an SVG document drawn for that.
std::tuple<uint32_t, uint32_t, uint32_t, SvgRequest::Placement> decodeBox(ExpoContentFit fit, uint32_t width,
                                                                          uint32_t height, bool downscale) {
  if (!downscale || fit == ExpoContentFit::None) return {0, 0, OXYOS_IMAGE_FIT_WITHIN, SvgRequest::Placement::Natural};
  bool covering = fit == ExpoContentFit::Cover || fit == ExpoContentFit::Fill;
  auto placement = fit == ExpoContentFit::ScaleDown ? SvgRequest::Placement::NaturalWithin : SvgRequest::Placement::Fitted;
  return {width, height, covering ? OXYOS_IMAGE_FIT_COVERING : OXYOS_IMAGE_FIT_WITHIN, placement};
}

bool isHash(const std::string& uri) {
  auto kind = expoImageSourceKind(uri);
  return kind == ExpoImageSourceKind::Blurhash || kind == ExpoImageSourceKind::Thumbhash;
}

}  // namespace

void Images::syncExpo(Tag tag, const View& entry, float scale) {
  auto props = std::static_pointer_cast<const ExpoImageProps>(entry.view.props);
  auto& shown = expo_[tag];
  auto frame = entry.view.layoutMetrics.frame.size;
  auto content = entry.view.layoutMetrics.getContentFrame().size;
  double viewWidth = static_cast<double>(gridPixels(frame.width, scale));
  double viewHeight = static_cast<double>(gridPixels(frame.height, scale));
  auto boxWidth = static_cast<uint32_t>(std::max(0L, gridPixels(content.width, scale)));
  auto boxHeight = static_cast<uint32_t>(std::max(0L, gridPixels(content.height, scale)));
  auto ref = expoImageRef(props->source);
  auto best = ref ? std::nullopt : bestExpoImageSource(expoImageSources(props->source), viewWidth, viewHeight);
  auto placeholder = bestExpoImageSource(expoImageSources(props->placeholder), viewWidth, viewHeight);
  if (shown.locked && !shown.reload) return;
  // cleanIfNeeded: a view with no area, or nothing to show, shows nothing.
  if (viewWidth <= 0 || viewHeight <= 0 || (!best && !ref && !placeholder)) {
    clearExpo(tag, shown);
    shown.recyclingKey = props->recyclingKey;
    return;
  }
  // A new recyclingKey (one given before and now) clears the view before its next source arrives.
  bool recycle = !shown.recyclingKey.empty() && !props->recyclingKey.empty() && props->recyclingKey != shown.recyclingKey;
  shown.recyclingKey = props->recyclingKey;
  auto fit = expoContentFit(props->contentFit, ExpoContentFit::Cover);
  auto [width, height, decodeFit, placement] = decodeBox(fit, boxWidth, boxHeight, props->allowDownscaling);
  auto keyOf = [&](const ExpoImageSource& source, bool forPlaceholder) {
    Key key{source.cacheKey.empty() ? source.uri : "key:" + source.cacheKey, width, height, decodeFit, placement, scale,
            true};
    if (forPlaceholder) {
      // PlaceholderDownsampleStrategy: a placeholder is decoded whole; a BlurHash at its own size.
      key.width = key.height = 0;
      key.placement = SvgRequest::Placement::Natural;
      if (isHash(source.uri)) key.width = static_cast<uint32_t>(source.width), key.height = static_cast<uint32_t>(source.height);
    }
    return key;
  };
  std::string mainIdentity = ref ? "ref:" + std::to_string(*ref) : best ? best->identity() : "";
  std::string placeholderIdentity = placeholder ? placeholder->identity() : "";
  bool changed = shown.reload || mainIdentity != shown.main.identity || placeholderIdentity != shown.placeholder.identity;
  if (!changed) {
    // The same source in a box of another size is decoded again for it, unless it is drawn at its
    // own size (fill and none keep the decode they have, as Android's onSizeChanged).
    if (!best || fit == ExpoContentFit::Fill || fit == ExpoContentFit::None || !props->allowDownscaling) return;
    Key key = keyOf(*best, false);
    if (key == shown.main.key || (shown.resized.loading && key == shown.resized.key)) return;
    cancel(shown.resized);
    if (shown.main.loading) {
      // Still on its way: asked again at the new size, still the same request for its events.
      cancel(shown.main);
      shown.main.key = key;
      request(tag, shown, shown.main, props->cachePolicy);
      return;
    }
    shown.resized = ExpoRequest{mainIdentity, *best, key, 0, static_cast<float>(best->scale)};
    request(tag, shown, shown.resized, props->cachePolicy);
    return;
  }
  shown.reload = false;
  if (recycle) {
    shown.drawn.clear();
    shown.fade.reset();
    tree_.setLayers(tag, nullptr);
  }
  for (auto* request : {&shown.main, &shown.placeholder, &shown.resized}) cancel(*request);
  shown.resized = ExpoRequest{};
  shown.told = false;
  shown.ref = ref;
  shown.main = ExpoRequest{mainIdentity};
  if (best) shown.main = ExpoRequest{mainIdentity, *best, keyOf(*best, false), 0, static_cast<float>(best->scale)};
  shown.placeholder = ExpoRequest{placeholderIdentity};
  if (placeholder) {
    shown.placeholder = ExpoRequest{placeholderIdentity, *placeholder, keyOf(*placeholder, true), 0,
                                    static_cast<float>(placeholder->scale)};
  }
  emit(tag, "loadStart", folly::dynamic::object());
  if (ref) {
    auto held = refs_.find(*ref);
    if (held != refs_.end() && held->second.picture) {
      // A decoded picture, used as it is (DecodedSource: no cache).
      expoLoaded(tag, shown, shown.main, held->second.picture, "none");
    } else {
      expoFailed(tag, shown, shown.main, "the ImageRef was released or has not loaded");
    }
  } else if (best) {
    request(tag, shown, shown.main, props->cachePolicy);
  }
  // Its placeholder, unless the source is already shown (a memory hit completes before the
  // thumbnail Glide starts beside it). Loaded with Glide's default caches, not the view's policy.
  if (placeholder && !shown.told) request(tag, shown, shown.placeholder, "memory-disk");
}

void Images::request(Tag tag, ExpoShown& shown, ExpoRequest& request, const std::string& cachePolicy) {
  bool memory = cachePolicy == "memory" || cachePolicy == "memory-disk";
  if (memory) {
    if (auto it = pictures_.find(request.key); it != pictures_.end()) {
      if (auto picture = it->second.lock()) return expoLoaded(tag, shown, request, picture, "memory");
    }
  }
  for (const auto& [id, load] : loads_) {
    if (load.key == request.key) {
      request.loading = id;
      return;
    }
  }
  request.loading = ++next_;
  auto kind = expoImageSourceKind(request.source.uri);
  loads_[request.loading] = Load{request.key, request.sourceScale, kind == ExpoImageSourceKind::Remote,
                                 kind == ExpoImageSourceKind::File, OXYOS_IMAGE_FROM_SOURCE, memory};
  std::string error;
  if (!start(request.loading, request.source, request.key, cachePolicy, error)) {
    loads_.erase(request.loading);
    request.loading = 0;
    expoFailed(tag, shown, request, error);
  }
}

void Images::cancel(ExpoRequest& request) {
  uint64_t id = std::exchange(request.loading, 0);
  release(id);
}

void Images::expoLoaded(Tag tag, ExpoShown& shown, ExpoRequest& request, const std::shared_ptr<const Picture>& picture,
                        const std::string& cacheType) {
  request.loading = 0;
  auto view = tree_.views().find(tag);
  if (view == tree_.views().end()) return;
  auto now = Clock::now();
  if (&request == &shown.resized) {
    // The source decoded at the view's new size takes the place of the one drawn, silently.
    for (auto& layer : shown.drawn) {
      if (!layer.placeholder) layer.picture = picture;
    }
    shown.main.key = request.key;
    request = ExpoRequest{};
    return publish(tag, shown, now);
  }
  if (&request == &shown.placeholder) {
    if (!shown.told) showPlaceholder(shown, request, picture, view->second.view);
    return publish(tag, shown, now);
  }
  bool first = !std::exchange(shown.told, true);
  showSource(shown, picture, cacheType, view->second.view);
  // The thumbnail is no longer wanted once the full request completes.
  cancel(shown.placeholder);
  publish(tag, shown, now);
  if (!first) return;
  // onResourceReady shows it (onDisplay), then the listener's onLoad: its source's URL, its size
  // in pixels, and whether it animates.
  emit(tag, "display", folly::dynamic::object());
  folly::dynamic source = folly::dynamic::object("url", shown.ref ? "" : request.source.uri)(
      "width", std::round(picture->naturalWidth * request.sourceScale))(
      "height", std::round(picture->naturalHeight * request.sourceScale))("mediaType", nullptr)(
      "isAnimated", picture->animated());
  emit(tag, "load", folly::dynamic::object("cacheType", cacheType)("source", std::move(source)));
}

void Images::expoFailed(Tag tag, ExpoShown& shown, ExpoRequest& request, const std::string& error) {
  request.loading = 0;
  const std::string& uri = request.source.uri;
  if (reported_.insert(uri).second) std::cerr << "[host] image " << uri << " not shown: " << error << "\n";
  if (&request == &shown.resized) {
    request = ExpoRequest{};
    return;
  }
  // A placeholder that fails says nothing (Glide's thumbnail has no listener); the view keeps what
  // it shows when its source does.
  if (&request == &shown.placeholder || std::exchange(shown.told, true)) return;
  emit(tag, "error", folly::dynamic::object("error", error));
}

void Images::showSource(ExpoShown& shown, const std::shared_ptr<const Picture>& picture, const std::string& cacheType,
                        const ShadowView& view) {
  auto props = std::static_pointer_cast<const ExpoImageProps>(view.props);
  auto now = Clock::now();
  ExpoLayer layer{.picture = picture, .placeholder = false, .ownFit = true,
                  .sourceWidth = picture->naturalWidth, .sourceHeight = picture->naturalHeight};
  bool initial = std::all_of(shown.drawn.begin(), shown.drawn.end(), [](const ExpoLayer& l) { return l.placeholder; });
  auto transition = expoTransition(props->transition);
  int duration = transition && transition->plays(cacheType, initial) ? transition->duration : 0;
  if (picture->animated() && props->autoplay && shown.animating) {
    layer.due = now + std::chrono::milliseconds(std::max<uint32_t>(10, picture->delays[0]));
  }
  if (duration <= 0 || shown.drawn.empty()) {
    // Nothing shown before: it fades in from nothing, or appears.
    if (duration > 0 && shown.drawn.empty()) {
      layer.from = 0;
      shown.drawn = {layer};
      shown.fade = ExpoShown::Fade{now, duration, transition->timing};
      return;
    }
    shown.drawn = {layer};
    shown.fade.reset();
    return;
  }
  // The one shown last fades out from where it is as the new one fades in over it.
  ExpoLayer previous = shown.drawn.back();
  if (shown.fade) {
    float t = std::chrono::duration<float, std::milli>(now - shown.fade->start).count() / shown.fade->duration;
    float share = expoTransitionCurve(shown.fade->timing, t);
    previous.from = previous.from + (previous.to - previous.from) * share;
  } else {
    previous.from = 1;
  }
  previous.to = 0;
  layer.from = 0;
  layer.to = 1;
  shown.drawn = {previous, layer};
  shown.fade = ExpoShown::Fade{now, duration, transition->timing};
}

void Images::showPlaceholder(ExpoShown& shown, const ExpoRequest& request, const std::shared_ptr<const Picture>& picture,
                             const ShadowView& view) {
  // Never over something shown, but another placeholder.
  for (const auto& layer : shown.drawn) {
    if (!layer.placeholder) return;
  }
  auto props = std::static_pointer_cast<const ExpoImageProps>(view.props);
  auto now = Clock::now();
  ExpoLayer layer{.picture = picture, .placeholder = true, .ownFit = !isHash(request.source.uri),
                  .sourceWidth = picture->naturalWidth, .sourceHeight = picture->naturalHeight};
  // A placeholder plays whatever autoplay says: it is a spinner more often than not.
  if (picture->animated()) layer.due = now + std::chrono::milliseconds(std::max<uint32_t>(10, picture->delays[0]));
  auto transition = expoTransition(props->transition);
  shown.drawn = {layer};
  shown.fade.reset();
  if (transition) {
    shown.drawn.front().from = 0;
    shown.fade = ExpoShown::Fade{now, transition->duration, transition->timing};
  }
}

void Images::clearExpo(Tag tag, ExpoShown& shown) {
  for (auto* request : {&shown.main, &shown.placeholder, &shown.resized}) {
    cancel(*request);
    *request = ExpoRequest{};
  }
  shown.ref.reset();
  shown.told = false;
  shown.drawn.clear();
  shown.fade.reset();
  tree_.setLayers(tag, nullptr);
}

void Images::publish(Tag tag, ExpoShown& shown, Clock::time_point now) {
  float share = 1;
  if (shown.fade) {
    float t = std::chrono::duration<float, std::milli>(now - shown.fade->start).count() / shown.fade->duration;
    share = expoTransitionCurve(shown.fade->timing, t);
    if (t >= 1) {
      // Its end: what faded out is gone, what faded in stays.
      shown.fade.reset();
      std::erase_if(shown.drawn, [](const ExpoLayer& layer) { return layer.to <= 0; });
      for (auto& layer : shown.drawn) layer.from = layer.to;
      share = 1;
    }
  }
  auto layers = std::make_shared<ExpoImageLayers>();
  folly::dynamic said = folly::dynamic::array;
  for (const auto& layer : shown.drawn) {
    float alpha = layer.from + (layer.to - layer.from) * share;
    layers->layers.push_back(ExpoImageLayer{layer.picture, layer.frame, alpha, layer.placeholder, layer.ownFit,
                                            layer.sourceWidth, layer.sourceHeight});
    said.push_back(folly::dynamic::object("alpha", alpha)("placeholder", layer.placeholder)(
        "frame", static_cast<int64_t>(layer.frame)));
  }
  tree_.setLayers(tag, layers);
  if (auto view = tree_.views().find(tag); view != tree_.views().end()) {
    Report::get().imageLayers(view->second.view, said);
  }
}

bool Images::tick(bool ready) {
  auto now = Clock::now();
  bool changed = false;
  bool fading = std::any_of(expo_.begin(), expo_.end(), [](const auto& entry) { return entry.second.fade.has_value(); });
  // A transition moves on once a frame of the display, when the window can take one.
  bool fadeDue = fading && ready && (!lastFade_ || now - *lastFade_ >= refresh_.period() - std::chrono::microseconds(500));
  if (fadeDue) lastFade_ = now;
  if (!fading) lastFade_.reset();
  for (auto& [tag, shown] : expo_) {
    bool moved = fadeDue && shown.fade;
    for (auto& layer : shown.drawn) {
      if (!layer.due || now < *layer.due || !layer.picture) continue;
      const Picture& picture = *layer.picture;
      // The next frame, as long as it has loops to play; it stops on its last.
      size_t next = layer.frame + 1;
      if (next >= picture.frames.size()) {
        ++layer.played;
        if (picture.loops != 0 && layer.played >= picture.loops) {
          layer.due.reset();
          continue;
        }
        next = 0;
      }
      layer.frame = next;
      auto delay = std::chrono::milliseconds(std::max<uint32_t>(10, picture.delays[next]));
      // Kept on its timeline, unless the loop fell far behind it.
      layer.due = now - *layer.due > delay ? now + delay : *layer.due + delay;
      moved = true;
    }
    if (moved) {
      publish(tag, shown, now);
      changed = true;
    }
  }
  return changed;
}

std::optional<Images::Clock::time_point> Images::nextFrame(bool ready) const {
  std::optional<Clock::time_point> next;
  auto sooner = [&](Clock::time_point due) {
    if (!next || due < *next) next = due;
  };
  for (const auto& [tag, shown] : expo_) {
    if (shown.fade && ready) sooner(lastFade_ ? *lastFade_ + refresh_.period() : Clock::now());
    for (const auto& layer : shown.drawn) {
      if (layer.due) sooner(*layer.due);
    }
  }
  return next;
}

void Images::emit(Tag tag, const char* event, folly::dynamic payload) {
  auto view = tree_.views().find(tag);
  if (view == tree_.views().end() || !view->second.view.eventEmitter) return;
  view->second.view.eventEmitter->dispatchEvent(event, folly::dynamic(payload));
  folly::dynamic extra = folly::dynamic::object;
  if (payload.count("cacheType")) {
    extra = folly::dynamic::object("cacheType", payload["cacheType"])("width", payload["source"]["width"])(
        "height", payload["source"]["height"]);
  } else if (payload.count("error")) {
    extra = folly::dynamic::object("error", payload["error"]);
  } else if (payload.count("loaded")) {
    extra = folly::dynamic::object("loaded", payload["loaded"])("total", payload["total"]);
  }
  Report::get().deliver(event, 0, view->second.view, extra);
}

void Images::setAnimating(Tag tag, bool animating) {
  auto it = expo_.find(tag);
  if (it == expo_.end()) return;
  it->second.animating = animating;
  auto now = Clock::now();
  for (auto& layer : it->second.drawn) {
    if (layer.placeholder || !layer.picture || !layer.picture->animated()) continue;
    if (!animating) {
      layer.due.reset();
    } else if (!layer.due) {
      // Resumed where it paused; one that had played all its loops starts again.
      if (layer.picture->loops != 0 && layer.played >= layer.picture->loops) layer.played = 0, layer.frame = 0;
      layer.due = now + std::chrono::milliseconds(std::max<uint32_t>(10, layer.picture->delays[layer.frame]));
    }
  }
}

void Images::setLocked(Tag tag, bool locked) {
  expo_[tag].locked = locked;
}

void Images::reload(Tag tag) {
  auto view = tree_.views().find(tag);
  if (view == tree_.views().end() || !isExpoImage(view->second.view)) return;
  expo_[tag].reload = true;
  syncExpo(tag, view->second, scale_);
}

void Images::expoPrefetch(const std::vector<std::string>& urls, const std::string& cachePolicy,
                          const std::vector<std::pair<std::string, std::string>>& headers,
                          std::function<void(bool)> done) {
  struct Waiting {
    size_t left;
    bool settled = false;
    std::function<void(bool)> done;
  };
  auto waiting = std::make_shared<Waiting>(Waiting{urls.size(), false, std::move(done)});
  auto settle = [waiting](bool fetched) {
    if (waiting->settled) return;
    if (!fetched || --waiting->left == 0) {
      waiting->settled = true;
      waiting->done(fetched);
    }
  };
  if (urls.empty()) return settle(true), void();
  for (const auto& url : urls) {
    auto resolved = resolveImageSource(url);
    if (!resolved.remote) {
      // Nothing to fetch: ready if it can be read.
      settle(resolved.data || (resolved.error.empty() && access(resolved.path.c_str(), R_OK) == 0));
      continue;
    }
    uint64_t id = ++next_;
    requests_[id] = Request{.uri = url, .fetched = [settle](const std::string& error) { settle(error.empty()); }};
    std::string error;
    // Into memory, and the disk unless the policy is memory alone, as Android's prefetch loads it.
    if (!fetchRemote(id, url, headers, OXYOS_IMAGE_CACHE_FORCE, true, cachePolicy != "memory",
                     OXYOS_IMAGE_WANT_PREFETCH, Key{}, "", error)) {
      requests_.erase(id);
      settle(false);
    }
  }
}

bool Images::clearCache(uint32_t caches) {
  // The decoded pictures kept for views to share are the memory cache too: one a view still draws
  // stays drawn, but is no longer found by another load.
  if (caches & OXYOS_IMAGE_IN_MEMORY) pictures_.clear();
  return oxyos_images_clear(loader_, caches) == 0;
}

std::optional<std::string> Images::cachePath(const std::string& key) {
  std::string path(4096, '\0');
  size_t length = 0;
  if (oxyos_images_cache_path(loader_, key.data(), key.size(), path.data(), path.size(), &length) != 0) {
    return std::nullopt;
  }
  path.resize(length);
  return path;
}

void Images::loadRef(const ExpoImageSource& source, uint32_t maxWidth, uint32_t maxHeight, std::optional<uint32_t> tint,
                     Referenced done) {
  uint64_t id = ++next_;
  int64_t ref = ++nextRef_;
  refs_[ref] = Ref{nullptr, tint, std::move(done)};
  refLoads_[id] = ref;
  // Fitted within the largest size asked for, never enlarged (Glide's centerInside).
  Key key{source.cacheKey.empty() ? source.uri : "key:" + source.cacheKey, maxWidth, maxHeight, OXYOS_IMAGE_FIT_WITHIN,
          SvgRequest::Placement::Fitted, scale_, true};
  auto kind = expoImageSourceKind(source.uri);
  loads_[id] = Load{key, static_cast<float>(source.scale), kind == ExpoImageSourceKind::Remote,
                    kind == ExpoImageSourceKind::File};
  std::string error;
  if (!start(id, source, key, "memory-disk", error)) {
    loads_.erase(id);
    refLoads_.erase(id);
    auto failed = std::move(refs_[ref].done);
    refs_.erase(ref);
    failed(0, nullptr, error);
  }
}

void Images::releaseRef(int64_t id) { refs_.erase(id); }

std::function<void(int64_t)> Images::releaser() {
  return [released = released_](int64_t id) {
    std::lock_guard<std::mutex> hold(released->lock);
    released->ids.push_back(id);
  };
}

}  // namespace oxyos
