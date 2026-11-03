#include "svg-image.h"

#include <include/core/SkCanvas.h>
#include <include/core/SkStream.h>
#include <include/core/SkSurface.h>
#include <modules/svg/include/SkSVGDOM.h>
#include <modules/svg/include/SkSVGRenderContext.h>
#include <modules/svg/include/SkSVGSVG.h>

#include <sys/eventfd.h>
#include <unistd.h>

#include <algorithm>
#include <cctype>
#include <cmath>
#include <fstream>
#include <stdexcept>
#include <system_error>
#include <thread>

namespace oxyos {

namespace {

// The largest picture made, in pixels: a side of 16384 as the compositor's buffers, and 4096 ×
// 4096 in all (64 MiB), an SVG wanted larger drawn at that and scaled.
constexpr double kMaxSide = 16384, kMaxPixels = 4096.0 * 4096.0;

std::string lower(std::string text) {
  for (auto& c : text) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
  return text;
}

// The first kilobyte of the file, or nullopt if it cannot be read.
std::optional<std::string> head(const std::string& path) {
  std::ifstream file(path, std::ios::binary);
  if (!file) return std::nullopt;
  std::string bytes(1024, '\0');
  file.read(bytes.data(), static_cast<std::streamsize>(bytes.size()));
  bytes.resize(static_cast<size_t>(file.gcount()));
  return bytes;
}

struct Document {
  sk_sp<SkSVGDOM> dom;
  float width = 0, height = 0;  // its natural size, 0 when it has none
  std::string error;
};

Document parse(const std::string& path) {
  Document document;
  auto stream = SkStream::MakeFromFile(path.c_str());
  if (!stream) {
    document.error = "cannot read the image: " + path;
    return document;
  }
  document.dom = SkSVGDOM::Builder().make(*stream);
  if (!document.dom || !document.dom->getRoot()) {
    document.dom = nullptr;
    document.error = "invalid SVG image: not a well-formed SVG document";
    return document;
  }
  // Its own width and height when both are lengths (CSS pixels, 96 to the inch, as a browser
  // takes them); else its view box's size, or the one length given and the view box's proportions.
  const SkSVGSVG& root = *document.dom->getRoot();
  SkSVGLengthContext lengths(SkSize::Make(0, 0), 96);
  auto absolute = [](const SkSVGLength& length) { return length.unit() != SkSVGLength::Unit::kPercentage; };
  float width = absolute(root.getWidth())
                    ? lengths.resolve(root.getWidth(), SkSVGLengthContext::LengthType::kHorizontal)
                    : 0;
  float height = absolute(root.getHeight())
                     ? lengths.resolve(root.getHeight(), SkSVGLengthContext::LengthType::kVertical)
                     : 0;
  const auto& box = root.getViewBox();
  if (box && !box->isEmpty()) {
    if (width <= 0 && height <= 0) {
      width = box->width(), height = box->height();
    } else if (width <= 0) {
      width = height * box->width() / box->height();
    } else if (height <= 0) {
      height = width * box->height() / box->width();
    }
  }
  if (width > 0 && height > 0) document.width = width, document.height = height;
  return document;
}

}  // namespace

bool svgByName(const std::string& path) {
  return path.size() > 4 && lower(path.substr(path.size() - 4)) == ".svg";
}

bool svgBySniffing(const std::string& bytes) {
  // Coil's SvgDecoder.isSvg: it starts with `<` and holds `<svg` in its first kilobyte; a byte
  // order mark and white space before the `<` are allowed, as XML allows them.
  size_t start = bytes.rfind("\xEF\xBB\xBF", 0) == 0 ? 3 : 0;
  while (start < bytes.size() && std::isspace(static_cast<unsigned char>(bytes[start]))) ++start;
  if (start >= bytes.size() || bytes[start] != '<') return false;
  return bytes.substr(0, 1024).find("<svg") != std::string::npos;
}

std::optional<std::pair<float, float>> svgImageSize(const std::string& path, std::string& error) {
  auto document = parse(path);
  if (!document.dom) {
    error = document.error;
    return std::nullopt;
  }
  if (document.width <= 0) {
    error = "the SVG image has no size of its own (no width, height or viewBox)";
    return std::nullopt;
  }
  return std::pair{document.width, document.height};
}

SvgImage decodeSvgImage(const std::string& path, const SvgRequest& request) {
  SvgImage out;
  auto document = parse(path);
  if (!document.dom) {
    out.error = document.error;
    return out;
  }
  // A document with no size of its own takes the box's.
  double naturalWidth = document.width, naturalHeight = document.height;
  if (naturalWidth <= 0) {
    naturalWidth = request.width / std::max(request.scale, 0.01f);
    naturalHeight = request.height / std::max(request.scale, 0.01f);
  }
  if (naturalWidth <= 0 || naturalHeight <= 0) {
    out.error = "the SVG image has no size (no width, height or viewBox) and its view none";
    return out;
  }
  out.naturalWidth = static_cast<float>(naturalWidth);
  out.naturalHeight = static_cast<float>(naturalHeight);
  // The size it is drawn at, in pixels, as the renderer places it (renderer.cpp's drawImage):
  // fitted to the box by its proportions, or at its natural size at the display's scale. A
  // vector picture is drawn at that size, larger than its own when the box is (never enlarged
  // from a smaller raster).
  double width, height;
  if (request.placement == SvgRequest::Placement::Fitted && request.width > 0 && request.height > 0) {
    double across = request.width / naturalWidth, down = request.height / naturalHeight;
    double s = request.covering ? std::max(across, down) : std::min(across, down);
    width = naturalWidth * s, height = naturalHeight * s;
  } else {
    width = naturalWidth * request.scale, height = naturalHeight * request.scale;
    if (request.placement == SvgRequest::Placement::NaturalWithin && request.width > 0 && request.height > 0) {
      double shrink = std::min({1.0, request.width / width, request.height / height});
      width *= shrink, height *= shrink;
    }
  }
  double limit = std::min({1.0, kMaxSide / width, kMaxSide / height, std::sqrt(kMaxPixels / (width * height))});
  width *= limit, height *= limit;
  int pixelsWide = std::max(1, static_cast<int>(std::lround(width)));
  int pixelsHigh = std::max(1, static_cast<int>(std::lround(height)));
  auto surface = SkSurfaces::Raster(SkImageInfo::MakeN32Premul(pixelsWide, pixelsHigh));
  if (!surface) {
    out.error = "the SVG image cannot be drawn at " + std::to_string(pixelsWide) + "x" + std::to_string(pixelsHigh);
    return out;
  }
  SkCanvas* canvas = surface->getCanvas();
  canvas->clear(SK_ColorTRANSPARENT);
  // The document laid out at its natural size (its percentages resolve against it) and scaled to
  // the pixels.
  canvas->scale(static_cast<float>(pixelsWide / naturalWidth), static_cast<float>(pixelsHigh / naturalHeight));
  document.dom->setContainerSize(SkSize::Make(static_cast<float>(naturalWidth), static_cast<float>(naturalHeight)));
  document.dom->render(canvas);
  out.image = surface->makeImageSnapshot();
  return out;
}

SvgDecoder::Shared::~Shared() {
  if (fd >= 0) close(fd);
}

SvgDecoder::SvgDecoder() : shared_(std::make_shared<Shared>()) {
  fd_ = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
  if (fd_ < 0) throw std::runtime_error("the SVG decoder cannot make its descriptor");
  shared_->fd = fd_;
}

SvgDecoder::~SvgDecoder() {
  // Loads still running end unheard: they write to a descriptor no longer read, closed once the
  // last of them is done (Shared's destructor).
  std::lock_guard guard(shared_->lock);
  shared_->answers.clear();
}

void SvgDecoder::load(uint64_t id, const std::string& path, const SvgRequest& request, bool sniffed,
                      const std::string& otherwise) {
  auto shared = shared_;
  auto run = [shared, id, path, request, sniffed, otherwise] {
    Answer answer{id, {}};
    if (sniffed) {
      auto bytes = head(path);
      if (!bytes || !svgBySniffing(*bytes)) answer.image.error = otherwise;
    }
    if (answer.image.error.empty()) answer.image = decodeSvgImage(path, request);
    std::lock_guard guard(shared->lock);
    shared->answers.push_back(std::move(answer));
    uint64_t one = 1;
    [[maybe_unused]] auto written = write(shared->fd, &one, sizeof one);
  };
  try {
    std::thread(run).detach();
  } catch (const std::system_error& error) {
    std::lock_guard guard(shared->lock);
    shared->answers.push_back(Answer{id, SvgImage{.error = std::string("the load cannot start: ") + error.what()}});
    uint64_t one = 1;
    [[maybe_unused]] auto written = write(shared->fd, &one, sizeof one);
  }
}

std::optional<SvgDecoder::Answer> SvgDecoder::answer() {
  std::lock_guard guard(shared_->lock);
  if (shared_->answers.empty()) {
    uint64_t drained;
    [[maybe_unused]] auto read_ = read(fd_, &drained, sizeof drained);
    return std::nullopt;
  }
  auto answer = std::move(shared_->answers.front());
  shared_->answers.pop_front();
  return answer;
}

}  // namespace oxyos
