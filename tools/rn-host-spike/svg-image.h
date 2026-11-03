// SVG documents as images (React Native's Image, which Bloom's images and expo-image's fallback
// draw): what Android's SVG decoders for its image loaders do (Coil's SvgDecoder, expo-image's
// Glide SVGDecoder), with Skia's SVG module (modules/svg, SkSVGDOM, as Chromium's and Android's
// tooling use it) in place of AndroidSVG. A document is parsed, rasterised at the size its view is
// drawn in pixels (the display's scale included, so it is as sharp at 1.25 as at 1), never at its
// own small size and enlarged, and given to the view as any decoded picture (images.h).
//
// A file is an SVG document when its name ends in `.svg`, or, as Coil sniffs it, when another
// decoder cannot read it and its first kilobyte starts with `<` and holds `<svg`. Its natural
// size, which `onLoad` reports and `center`, `repeat` and `none` draw at, is the root's `width`
// and `height` when they are lengths, else its `viewBox`'s size (Coil's default), else the box's.
// Text inside an SVG image is not drawn (no font manager is given to the document); links to
// other files (`<image href>`) are not followed.
#pragma once

#include <include/core/SkImage.h>

#include <cstdint>
#include <deque>
#include <memory>
#include <mutex>
#include <optional>
#include <string>

namespace oxyos {

// How the picture is fitted to its view's content box, as images.h asks for it.
struct SvgRequest {
  uint32_t width = 0, height = 0;  // the box, in pixels (0: its natural size)
  bool covering = false;           // cover and stretch cover the box, the others fit within it
  // center and repeat draw it at its natural size, shrunk to fit; none at its natural size.
  enum class Placement : uint8_t { Fitted, NaturalWithin, Natural } placement = Placement::Fitted;
  float scale = 1;  // the display's, for the natural placements
};

struct SvgImage {
  sk_sp<SkImage> image;  // premultiplied, sRGB; null with `error`
  float naturalWidth = 0, naturalHeight = 0;
  std::string error;
};

// Whether the file's name says it is an SVG document.
bool svgByName(const std::string& path);
// Whether the bytes look like an SVG document (Coil's sniffing).
bool svgBySniffing(const std::string& head);

// The document's natural size in logical pixels, or nullopt with `error`.
std::optional<std::pair<float, float>> svgImageSize(const std::string& path, std::string& error);

// Parses and rasterises the document; on the caller's thread.
SvgImage decodeSvgImage(const std::string& path, const SvgRequest& request);

// Decodes SVG documents on threads of their own, as the image loader (oxyos_images_*) decodes
// PNG and JPEG; its descriptor is readable while answers wait.
class SvgDecoder {
 public:
  struct Answer {
    uint64_t id = 0;
    SvgImage image;
  };

  // Throws std::runtime_error if its descriptor cannot be made.
  SvgDecoder();
  ~SvgDecoder();
  SvgDecoder(const SvgDecoder&) = delete;
  SvgDecoder& operator=(const SvgDecoder&) = delete;

  // Starts decoding `path`. With `sniffed`, the file is first read to see whether it is an SVG
  // document; if it is not, the answer is `otherwise`, the error another decoder gave.
  void load(uint64_t id, const std::string& path, const SvgRequest& request, bool sniffed = false,
            const std::string& otherwise = "");
  // The next answer, if any; never blocks.
  std::optional<Answer> answer();
  int fd() const { return fd_; }

 private:
  struct Shared {
    std::mutex lock;
    std::deque<Answer> answers;
    int fd = -1;
    ~Shared();  // closes `fd`, once no load still running holds it
  };
  std::shared_ptr<Shared> shared_;
  int fd_;
};

}  // namespace oxyos
