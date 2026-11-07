// React Native's Image on OxyOS (docs/react-native-android-adaptation.mdx): what Android's
// ReactImageView does for a mounted Image view. Each view's source is loaded at the size it is
// drawn (crates/oxyos-image through oxyos_client.h, decoded on threads of their own), the picture is
// given to the view (ViewTree::setPicture), and the view's load events follow Android's: only
// when JavaScript listens (shouldNotifyLoadEvents), loadStart, then load or error, then loadEnd,
// once for each source a view shows. A view drawn at another size shows its source decoded again
// for that size with no events, as Android's view resized keeps its request: its source did not
// change. react-native-svg's `<Image href>` (RNSVGImage) loads the same way, at the size it is
// drawn through its SVG view's view box, its picture drawn by svg.cpp, and says onLoad as its
// Android ImageView does.
//
// Sources are files: `file://` URIs (a release bundle's assets, which React Native resolves to
// the bundle's drawable folders when it is loaded from the file system) and absolute paths; and
// remote images (`http`, `https`), as Android's Fresco loads them over OkHttp: fetched with the
// application's network (the one its `fetch` uses, network.h: its TLS, proxy and cookies), the
// source's `headers` sent and its `cache` followed (`default`, `reload`, `force-cache`,
// `only-if-cached`), through a memory cache and an HTTP cache on disk in the application's cache
// directory (`images/` in it, 40 MiB as Fresco's, crates/oxyos-image's `remote` and
// crates/oxyos-net's `cache`), its `progress` told as it arrives when its length is known. An
// application without the `network` capability (docs/adr/0030-installing-applications.mdx,
// decision 8) gets no remote image, which fails with that reason. A view's load is cancelled
// when it unmounts or its source changes, and its request stops once no load waits for it.
// `data:` URIs are decoded by crates/oxyos-image (base64 or percent-encoded; an SVG one by Skia's
// SVG module), as Android's Fresco decodes them. PNG, JPEG, WebP and GIF (an
// animated one's first frame) are decoded by crates/oxyos-image; SVG documents, files or fetched,
// by Skia's SVG module, rasterised at the size the view is drawn in pixels (svg-image.h), as
// Android's image loaders' SVG decoders do.
//
// React Native's ImageLoader module (Image.getSize, getSizeWithHeaders, prefetch, queryCache,
// host-modules.cpp) asks here for remote images: their size, fetched; a prefetch into the disk
// cache, as Android's prefetchToDiskCache; where each is cached.
//
// expo-image's views (expo-image.h) load here too, through the same loader and caches, as its
// Android view (ExpoImageViewWrapper) loads through Glide: the source chosen among several by the
// view's size (getBestSource), its placeholder (any source, a BlurHash or a ThumbHash decoded by
// oxyos-image) shown until it arrives, files, `require()` assets, data: URIs, remote images with
// their headers and cacheKey and SVG documents alike. Its cachePolicy is Glide's: `none` uses no
// cache, `disk` (the default) the disk cache alone, `memory` the memory cache alone, `memory-disk`
// both; a cached image is used however old (Glide's caches keep no HTTP freshness), and a decoded
// picture another view shows is a memory hit when the memory cache is used. Its events are
// Android's, each once per source: onLoadStart, onProgress (a remote image's length known),
// then onDisplay and onLoad ({cacheType, source: {url, width, height, mediaType, isAnimated}}, the
// size in pixels) or onError ({error}); expo-image's JavaScript says onLoadEnd after either. A view
// resized decodes its source again at the new size and says nothing more. The picture arriving
// cross-dissolves with what the view showed (a placeholder, or the source before) over the
// transition's duration and curve (expo-image.h), skipped on a cache hit when skipOnCacheHit says
// so for a first image; a placeholder fades in from nothing. A recyclingKey changed clears the view
// before the next source arrives. An animated GIF or WebP plays with its frames' timing as many
// times as it loops, unless autoplay is false (a placeholder always plays); startAnimating and
// stopAnimating pause and resume it. A locked view (lockResourceAsync) loads nothing new until
// unlocked or reloaded. `priority` is accepted: every load starts at once, so there is no queue for
// it to order. `svgVariables` and SF Symbols are not supported.
//
// expo-image's module (host-modules.cpp's OxyOSExpoImage) asks here for Image.prefetch (into the
// caches its cachePolicy names), clearMemoryCache, clearDiskCache, getCachePathAsync and loadAsync,
// whose ImageRef is a picture held here by its id until released, shown by any view given it.
#pragma once

#include "display-refresh.h"
#include "expo-image.h"
#include "renderer.h"
#include "svg-image.h"

#include <oxyos_client.h>

#include <react/renderer/core/ReactPrimitives.h>
#include <react/renderer/imagemanager/primitives.h>

#include <chrono>
#include <cstdint>
#include <functional>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <set>
#include <string>
#include <tuple>
#include <vector>

namespace oxyos {

// The file a source URI names, or that it is remote (`http`, `https`: `path` is the URI), or why
// there is none.
struct ResolvedSource {
  std::string path;
  std::string error;
  bool remote = false;
  bool data = false;  // a data: URI (RFC 2397), `path` the URI, decoded by oxyos-image
};
ResolvedSource resolveImageSource(const std::string& uri);

class Images {
 public:
  using Clock = std::chrono::steady_clock;

  // Throws std::runtime_error if the loader cannot start. Transitions are paced by `refresh`.
  Images(ViewTree& tree, const DisplayRefresh& refresh);
  ~Images();
  Images(const Images&) = delete;
  Images& operator=(const Images&) = delete;

  // After the tree changed, or the display's scale: starts loading what each Image view shows,
  // at the size it is drawn. A view keeps its picture until the next one arrives.
  void sync(float scale);
  // Fetches remote images from now on with `net`, the application's network, through a disk cache
  // in `cacheDirectory` (none for "": memory only). Without it remote images fail.
  void connect(OxyOSNet* net, const std::string& cacheDirectory);

  // Each turn: gives views the pictures decoded and sends their events. True if any changed.
  bool deliver();

  // For React Native's ImageLoader, on the loop's thread, each answered there by `deliver`:
  // a remote image's or a data: URI's size in pixels (Image.getSize, getSizeWithHeaders), or why
  // there is none.
  using Sized = std::function<void(double width, double height, const std::string& error)>;
  void size(const std::string& uri, const std::vector<std::pair<std::string, std::string>>& headers, Sized done);
  // A remote image fetched into the disk cache (Image.prefetch, `requestId` its abortRequest's).
  using Fetched = std::function<void(const std::string& error)>;
  void prefetch(const std::string& uri, int64_t requestId, Fetched done);
  void abort(int64_t requestId);
  // Where each remote image is cached: "memory" or "disk", as Android's queryCache; none if neither.
  std::optional<std::string> cached(const std::string& uri);
  // Readable while decoded pictures wait to be given (either decoder's).
  int fd() const { return epoll_; }

  // Each turn: moves expo-image views' transitions and animated pictures on. A transition moves
  // once a frame of the display, when the window can take one (`ready`); an animation's frame is
  // shown when it is due. True if any view draws something new.
  bool tick(bool ready);
  // When tick has something to do next, if anything.
  std::optional<Clock::time_point> nextFrame(bool ready) const;

  // expo-image's module. Image.prefetch: the URLs fetched into the caches `cachePolicy` names
  // ("memory": memory alone; else memory and disk), `done` told true once all are, false at the
  // first that fails.
  void expoPrefetch(const std::vector<std::string>& urls, const std::string& cachePolicy,
                    const std::vector<std::pair<std::string, std::string>>& headers, std::function<void(bool)> done);
  // clearMemoryCache, clearDiskCache: true if there was a cache to clear.
  bool clearCache(uint32_t caches);
  // getCachePathAsync: the file holding the image cached by `key` (a URL or a cacheKey).
  std::optional<std::string> cachePath(const std::string& key);
  // loadAsync: the source's picture, within maxWidth × maxHeight pixels when given (0: its own
  // size), tinted when asked (0: not), held as an ImageRef: `done` gets its id and picture, or an
  // error.
  using Referenced = std::function<void(int64_t id, std::shared_ptr<const Picture> picture, const std::string& error)>;
  void loadRef(const ExpoImageSource& source, uint32_t maxWidth, uint32_t maxHeight, std::optional<uint32_t> tint,
               Referenced done);
  // An ImageRef released (its release()).
  void releaseRef(int64_t id);
  // What releases one from any thread (its JavaScript object collected): taken on the next deliver.
  std::function<void(int64_t)> releaser();
  // The view functions: startAnimating and stopAnimating, lockResourceAsync and
  // unlockResourceAsync, reloadAsync.
  void setAnimating(facebook::react::Tag tag, bool animating);
  void setLocked(facebook::react::Tag tag, bool locked);
  void reload(facebook::react::Tag tag);

 private:
  // A picture as loaded: the file, the box in pixels and how it fits it (oxyos_images_load), and,
  // for an SVG document drawn at its natural size (center, repeat, none), how and at what scale.
  // An expo-image view's picture is decoded with every frame of an animated one (`animated`).
  struct Key {
    std::string path;
    uint32_t width = 0, height = 0, fit = 0;
    SvgRequest::Placement placement = SvgRequest::Placement::Fitted;
    float scale = 1;
    bool animated = false;
    bool operator<(const Key& other) const {
      return std::tie(path, width, height, fit, placement, scale, animated) <
             std::tie(other.path, other.width, other.height, other.fit, other.placement, other.scale, other.animated);
    }
    bool operator==(const Key& other) const = default;
    SvgRequest svg() const {
      return SvgRequest{width, height, fit == OXYOS_IMAGE_FIT_COVERING, placement, scale};
    }
  };
  // A load on its way: what it loads and its source's scale.
  // `file` for a file oxyos-image may fail to read that may be an SVG document; `from` where its
  // bytes came from (OXYOS_IMAGE_FROM_*), kept while the SVG decoder decodes them.
  struct Load {
    Key key;
    float sourceScale = 1;
    bool remote = false;
    bool file = false;
    uint32_t from = OXYOS_IMAGE_FROM_SOURCE;
    // Whether its picture is kept for other views to share (it used the memory cache).
    bool memory = true;
  };
  // An ImageLoader request on its way (`size`, `prefetch`).
  struct Request {
    std::string uri;
    Sized sized;
    Fetched fetched;
    int64_t requestId = 0;
  };
  // What a view shows, or is loading.
  struct Shown {
    Key key;
    std::string uri;
    bool remote = false;
    float sourceScale = 1;
    uint64_t loading = 0;  // the load it waits for, 0 if none
    bool told = false;     // its source's load or error was told (loadEnd sent)
  };

  // What an expo-image view shows, and is loading (expo-image.h, ExpoImageViewWrapper's state).
  struct ExpoLayer {
    std::shared_ptr<const Picture> picture;
    bool placeholder = false, ownFit = true;
    float sourceWidth = 0, sourceHeight = 0;
    // Its alpha going from `from` to `to` while the view's fade runs.
    float from = 1, to = 1;
    // An animated picture's frame shown, when the next is due, and the loops played.
    size_t frame = 0;
    std::optional<Clock::time_point> due;
    uint32_t played = 0;
  };
  struct ExpoRequest {
    std::string identity;  // what it loads ("" for nothing)
    ExpoImageSource source;
    Key key;
    uint64_t loading = 0;
    float sourceScale = 1;
  };
  struct ExpoShown {
    ExpoRequest main, placeholder;
    // A decode of the main source at another size, after the view resized: given silently.
    ExpoRequest resized;
    std::optional<int64_t> ref;  // an ImageRef shown as its source
    std::string recyclingKey;
    bool told = false;  // the main source's load or error was told
    std::vector<ExpoLayer> drawn;
    struct Fade {
      Clock::time_point start;
      int duration = 0;
      ExpoTransition::Timing timing = ExpoTransition::Timing::EaseInOut;
    };
    std::optional<Fade> fade;
    bool animating = true;  // startAnimating / stopAnimating
    bool locked = false, reload = false;
  };
  // An ImageRef (loadAsync) on its way or held.
  struct Ref {
    std::shared_ptr<const Picture> picture;
    std::optional<uint32_t> tint;
    Referenced done;
  };

  // Starts fetching a remote image for `id`; false with `error` if it cannot start.
  bool fetch(uint64_t id, const std::string& uri, const facebook::react::ImageSource& source, uint32_t want,
             const Key& key, std::string& error);
  // The fetch itself, with how it uses the caches (OxyOSImageFetch's), `key` the caches' (empty: the
  // URL).
  bool fetchRemote(uint64_t id, const std::string& uri, const std::vector<std::pair<std::string, std::string>>& headers,
                   uint32_t cache, bool memory, bool disk, uint32_t want, const Key& key, const std::string& cacheKey,
                   std::string& error);
  // Starts a load of an expo-image source (any kind) for `id`, as `cachePolicy` uses the caches;
  // false with `error` if it cannot start.
  bool start(uint64_t id, const ExpoImageSource& source, const Key& key, const std::string& cachePolicy,
             std::string& error);
  // Whether anything still waits for the load `id`.
  bool waited(uint64_t id) const;
  void syncExpo(facebook::react::Tag tag, const View& entry, float scale);
  // Starts the request a view's source or placeholder makes (sharing a load on its way, or a
  // picture shown, when its cache policy uses memory); a picture at hand is given at once.
  void request(facebook::react::Tag tag, ExpoShown& shown, ExpoRequest& request, const std::string& cachePolicy);
  // A picture for an expo-image view's source, placeholder or resized decode, or its error.
  void expoLoaded(facebook::react::Tag tag, ExpoShown& shown, ExpoRequest& request,
                  const std::shared_ptr<const Picture>& picture, const std::string& cacheType);
  void expoFailed(facebook::react::Tag tag, ExpoShown& shown, ExpoRequest& request, const std::string& error);
  // ExpoImageViewWrapper.onResourceReady: the source's picture cross-dissolving with what was shown,
  // or a placeholder shown when nothing else is.
  void showSource(ExpoShown& shown, const std::shared_ptr<const Picture>& picture, const std::string& cacheType,
                  const facebook::react::ShadowView& view);
  void showPlaceholder(ExpoShown& shown, const ExpoRequest& request, const std::shared_ptr<const Picture>& picture,
                       const facebook::react::ShadowView& view);
  // Stops what a view waits for and shows nothing (cleanIfNeeded).
  void clearExpo(facebook::react::Tag tag, ExpoShown& shown);
  void cancel(ExpoRequest& request);
  // Gives the view what it draws now, as ExpoImageLayers.
  void publish(facebook::react::Tag tag, ExpoShown& shown, Clock::time_point now);
  void emit(facebook::react::Tag tag, const char* event, folly::dynamic payload);
  // A picture from an answer's pixels: `frames` frames of width × height after each other.
  static std::shared_ptr<Picture> picture(const OxyOSImageAnswer& answer, float sourceScale);
  // A load no view waits for any more: cancelled, its request stopped if no other load waits.
  void release(uint64_t id);
  // What arrived of a remote image, told to the views waiting for it.
  void progressed(uint64_t id, uint64_t loaded, uint64_t total);
  // An ImageLoader request answered.
  void requestAnswered(uint64_t id, const OxyOSImageAnswer& answer);
  bool notifies(const facebook::react::ShadowView& view) const;
  void started(facebook::react::Tag tag);
  // A load answered: its picture to the views waiting for it, or its error.
  void answered(uint64_t id, std::shared_ptr<const Picture> picture, const std::string& error);
  // Where a load's bytes came from, as expo-image's cacheType says it.
  static std::string cacheType(uint32_t from);
  // Each gives the view its picture (or none) and tells JavaScript, once for its source.
  void loaded(facebook::react::Tag tag, Shown& shown, const std::shared_ptr<const Picture>& picture);
  void failed(facebook::react::Tag tag, Shown& shown, const std::string& error);

  ViewTree& tree_;
  const DisplayRefresh& refresh_;
  OxyOSImages* loader_;
  SvgDecoder svg_;
  int epoll_ = -1;  // readable when either decoder's descriptor is
  uint64_t next_ = 0;
  std::map<facebook::react::Tag, Shown> shown_;
  std::map<uint64_t, Load> loads_;
  std::map<uint64_t, Request> requests_;
  // Whether remote images can be fetched: a network connected (the `network` capability is the
  // launch's, LaunchCapabilities).
  bool connected_ = false;
  // Pictures shown, shared by the views that show the same one, while any does.
  std::map<Key, std::weak_ptr<const Picture>> pictures_;
  std::set<std::string> reported_;  // URIs whose failure was logged
  // expo-image's views, their requests by load, ImageRefs (held, and loading by load), and the
  // scale views were last laid out at.
  std::map<facebook::react::Tag, ExpoShown> expo_;
  std::map<int64_t, Ref> refs_;
  std::map<uint64_t, int64_t> refLoads_;
  int64_t nextRef_ = 0;
  struct Released {
    std::mutex lock;
    std::vector<int64_t> ids;
  };
  std::shared_ptr<Released> released_ = std::make_shared<Released>();
  float scale_ = 1;
  // When transitions last moved on, once a frame of the display.
  std::optional<Clock::time_point> lastFade_;
};

}  // namespace oxyos
