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
// the bundle's drawable folders when it is loaded from the file system) and absolute paths.
// Remote (`http`, `https`) and `data:` sources fail with a message: OxyOS does not load them yet.
// PNG and JPEG are decoded by crates/oxyos-image; SVG documents by Skia's SVG module, rasterised
// at the size the view is drawn in pixels (svg-image.h), as Android's image loaders' SVG decoders
// do.
#pragma once

#include "renderer.h"
#include "svg-image.h"

#include <oxyos_client.h>

#include <react/renderer/core/ReactPrimitives.h>

#include <cstdint>
#include <map>
#include <memory>
#include <optional>
#include <set>
#include <string>
#include <tuple>

namespace oxyos {

// The file a source URI names, or why there is none.
struct ResolvedSource {
  std::string path;
  std::string error;
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
  // Each turn: gives views the pictures decoded and sends their events. True if any changed.
  bool deliver();
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
  };
  // What a view shows, or is loading.
  struct Shown {
    Key key;
    std::string uri;
    float sourceScale = 1;
    uint64_t loading = 0;  // the load it waits for, 0 if none
    bool told = false;     // its source's load or error was told (loadEnd sent)
  };

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
  // Pictures shown, shared by the views that show the same one, while any does.
  std::map<Key, std::weak_ptr<const Picture>> pictures_;
  std::set<std::string> reported_;  // URIs whose failure was logged
};

}  // namespace oxyos
