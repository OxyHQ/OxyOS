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
// `data:` sources fail with a message: OxyOS does not load them yet. PNG, JPEG, WebP and GIF (an
// animated one's first frame) are decoded by crates/oxyos-image; SVG documents, files or fetched,
// by Skia's SVG module, rasterised at the size the view is drawn in pixels (svg-image.h), as
// Android's image loaders' SVG decoders do.
//
// React Native's ImageLoader module (Image.getSize, getSizeWithHeaders, prefetch, queryCache,
// host-modules.cpp) asks here for remote images: their size, fetched; a prefetch into the disk
// cache, as Android's prefetchToDiskCache; where each is cached.
#pragma once

#include "renderer.h"
#include "svg-image.h"

#include <oxyos_client.h>

#include <react/renderer/core/ReactPrimitives.h>
#include <react/renderer/imagemanager/primitives.h>

#include <cstdint>
#include <functional>
#include <map>
#include <memory>
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
};
ResolvedSource resolveImageSource(const std::string& uri);

class Images {
 public:
  // Throws std::runtime_error if the loader cannot start.
  explicit Images(ViewTree& tree);
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
  // a remote image's size in pixels (Image.getSize, getSizeWithHeaders), or why there is none.
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

 private:
  // A picture as loaded: the file, the box in pixels and how it fits it (oxyos_images_load), and,
  // for an SVG document drawn at its natural size (center, repeat, none), how and at what scale.
  struct Key {
    std::string path;
    uint32_t width = 0, height = 0, fit = 0;
    SvgRequest::Placement placement = SvgRequest::Placement::Fitted;
    float scale = 1;
    bool operator<(const Key& other) const {
      return std::tie(path, width, height, fit, placement, scale) <
             std::tie(other.path, other.width, other.height, other.fit, other.placement, other.scale);
    }
    bool operator==(const Key& other) const = default;
    SvgRequest svg() const {
      return SvgRequest{width, height, fit == OXYOS_IMAGE_FIT_COVERING, placement, scale};
    }
  };
  // A load on its way: what it loads and its source's scale.
  struct Load {
    Key key;
    float sourceScale = 1;
    bool remote = false;
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

  // Starts fetching a remote image for `id`; false with `error` if it cannot start.
  bool fetch(uint64_t id, const std::string& uri, const facebook::react::ImageSource& source, uint32_t want,
             const Key& key, std::string& error);
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
  // Each gives the view its picture (or none) and tells JavaScript, once for its source.
  void loaded(facebook::react::Tag tag, Shown& shown, const std::shared_ptr<const Picture>& picture);
  void failed(facebook::react::Tag tag, Shown& shown, const std::string& error);

  ViewTree& tree_;
  OxyOSImages* loader_;
  SvgDecoder svg_;
  int epoll_ = -1;  // readable when either decoder's descriptor is
  uint64_t next_ = 0;
  std::map<facebook::react::Tag, Shown> shown_;
  std::map<uint64_t, Load> loads_;
  std::map<uint64_t, Request> requests_;
  // Whether remote images are fetched: with a network connected, and the `network` capability.
  bool connected_ = false;
  bool networkAllowed_ = false;
  // Pictures shown, shared by the views that show the same one, while any does.
  std::map<Key, std::weak_ptr<const Picture>> pictures_;
  std::set<std::string> reported_;  // URIs whose failure was logged
};

}  // namespace oxyos
