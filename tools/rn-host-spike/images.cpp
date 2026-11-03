#include "images.h"

#include "report.h"
#include "system.h"

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
  auto scheme = uri.substr(0, uri.find(':'));
  for (auto& c : scheme) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
  if (scheme == "http" || scheme == "https") return {uri, "", true};
  if (uri.rfind("data:", 0) == 0) return {"", "data: images are not supported on OxyOS yet"};
  return {"", "no file for the image source " + uri};
}

namespace {

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

// What an application without the network is told (network.cpp says the same of its requests).
constexpr const char* kNoNetwork = "this application was not granted the network";

}  // namespace

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

void Images::connect(OxyOSNet* net, const std::string& cacheDirectory) {
  // An installed version reaches the Internet only if its account granted `network`
  // (docs/adr/0030-installing-applications.mdx, decision 8), images included.
  networkAllowed_ = LaunchCapabilities::fromEnvironment().allows("network");
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
  if (!networkAllowed_) {
    error = kNoNetwork;
    return false;
  }
  if (!connected_) {
    error = "remote images need the network, which this host has not";
    return false;
  }
  std::vector<OxyOSNetHeader> headers;
  for (const auto& [name, value] : source.headers) {
    headers.push_back({name.data(), name.size(), value.data(), value.size()});
  }
  OxyOSImageFetch request{
      .url = uri.data(),
      .url_length = uri.size(),
      .headers = headers.data(),
      .header_count = headers.size(),
      .cache = cachePolicy(source.cache),
      .memory = 1,
      .disk = 1,
      .want = want,
      .width = key.width,
      .height = key.height,
      .fit = key.fit,
  };
  if (oxyos_images_fetch(loader_, id, &request) != 0) {
    error = oxyos_last_error();
    return false;
  }
  Report::get().image("fetch", uri);
  return true;
}

void Images::release(uint64_t id) {
  if (!id || !loads_.count(id)) return;
  for (const auto& [tag, shown] : shown_) {
    if (shown.loading == id) return;
  }
  bool remote = loads_[id].remote;
  std::string uri = loads_[id].key.path;
  loads_.erase(id);
  oxyos_images_cancel(loader_, id);
  if (remote) Report::get().image("cancelled", uri);
}

void Images::sync(float scale) {
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
    if (resolved.remote && !networkAllowed_) resolved.error = kNoNetwork;
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
      loads_[shown.loading] = Load{shown.key, shown.sourceScale, shown.remote};
      const auto& path = shown.key.path;
      std::string error;
      bool begun = true;
      if (shown.remote) {
        begun = fetch(shown.loading, path, *source, OXYOS_IMAGE_WANT_PICTURE, shown.key, error);
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
      // A remote image oxyos-image does not decode: an SVG document by its type, or as Coil sniffs
      // it; the SVG decoder looks, and answers with this error if it is not one.
      std::string type(answer.content_type ? answer.content_type : "", answer.content_type_length);
      bool svg = type.find("image/svg") != std::string::npos;
      auto bytes = std::make_shared<const std::string>(reinterpret_cast<const char*>(answer.data), answer.data_length);
      svg_.load(answer.id, SvgSource(load->second.key.path, bytes), load->second.key.svg(), !svg, error);
      continue;
    }
    if (answer.kind == OXYOS_IMAGE_FAILED && !load->second.remote) {
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

void Images::progressed(uint64_t id, uint64_t loaded, uint64_t total) {
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
  if (!fetch(id, uri, source, OXYOS_IMAGE_WANT_SIZE, Key{}, error)) {
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
  if (answer.kind == OXYOS_IMAGE_SIZE) return request.sized(answer.natural_width, answer.natural_height, "");
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
