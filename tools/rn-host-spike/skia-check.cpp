// Skia's smoke test: the libraries build.sh builds link and draw. One scene (a rounded rectangle on
// a blurred shadow, a linear gradient, text in a variable font at two weights, a line shaped by
// HarfBuzz, an SVG and a drawable defined here) is drawn on the CPU into BGRA pixels the program
// owns, then, where a Vulkan loader and device are present, by Ganesh on Vulkan; the two are read
// back and compared.
//
//   oxyos-skia-check <variable-font.ttf> <out-dir>
//
// Writes <out-dir>/raster.png, and <out-dir>/vulkan.png when there is a device. Exits non-zero when
// something does not draw, or when the two differ by more than antialiasing does.

#include "include/core/SkBlurTypes.h"
#include "include/core/SkCanvas.h"
#include "include/core/SkColorSpace.h"
#include "include/core/SkDrawable.h"
#include "include/core/SkFont.h"
#include "include/core/SkFontArguments.h"
#include "include/core/SkFontMgr.h"
#include "include/core/SkFontMetrics.h"
#include "include/core/SkMaskFilter.h"
#include "include/core/SkPaint.h"
#include "include/core/SkPixmap.h"
#include "include/core/SkRRect.h"
#include "include/core/SkStream.h"
#include "include/core/SkSurface.h"
#include "include/core/SkTextBlob.h"
#include "include/core/SkTypeface.h"
#include "include/effects/SkGradient.h"
#include "include/encode/SkPngEncoder.h"
#include "include/gpu/ganesh/GrDirectContext.h"
#include "include/gpu/ganesh/SkSurfaceGanesh.h"
#include "include/gpu/ganesh/vk/GrVkDirectContext.h"
#include "include/gpu/vk/VulkanBackendContext.h"
#include "include/gpu/vk/VulkanExtensions.h"
#include "include/gpu/vk/VulkanPreferredFeatures.h"
#include "include/ports/SkFontMgr_empty.h"
#include "modules/skshaper/include/SkShaper.h"
#include "modules/skshaper/include/SkShaper_harfbuzz.h"
#include "modules/skunicode/include/SkUnicode_icu.h"
#include "modules/svg/include/SkSVGDOM.h"
// Skia's Vulkan Memory Allocator behind its VulkanMemoryAllocator interface, as Skia's own tools
// make it: the backend context must carry an allocator.
#include "src/gpu/GpuTypesPriv.h"
#include "src/gpu/vk/vulkanmemoryallocator/VulkanMemoryAllocatorPriv.h"

#include <dlfcn.h>

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

namespace {

constexpr int kWidth = 480;
constexpr int kHeight = 320;
constexpr SkColor kBackground = 0xFFF4F4F8;

bool fail(const char* what) {
  std::fprintf(stderr, "oxyos-skia-check: %s\n", what);
  return false;
}

// A drawable defined outside Skia: its typeinfo needs SkDrawable's, which Skia emits only when it
// is built with RTTI, as the host is (build.sh).
class Badge final : public SkDrawable {
  SkRect onGetBounds() override { return SkRect::MakeWH(24, 24); }
  void onDraw(SkCanvas* canvas) override {
    SkPaint paint;
    paint.setAntiAlias(true);
    paint.setColor(0xFFE0457B);
    canvas->drawCircle(12, 12, 12, paint);
  }
};

struct Fonts {
  sk_sp<SkFontMgr> manager;
  sk_sp<SkTypeface> regular;  // wght 400
  sk_sp<SkTypeface> bold;     // wght 700
};

sk_sp<SkTypeface> at_weight(const SkFontMgr& manager, const char* path, float weight) {
  const SkFontArguments::VariationPosition::Coordinate wght[] = {
      {SkSetFourByteTag('w', 'g', 'h', 't'), weight}};
  SkFontArguments arguments;
  arguments.setVariationDesignPosition({wght, 1});
  return manager.makeFromStream(SkStream::MakeFromFile(path), arguments);
}

const char kSvg[] =
    "<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64' viewBox='0 0 64 64'>"
    "<path d='M8 32 L32 8 L56 32 L32 56 Z' fill='#2F7DE1' stroke='#123' stroke-width='3'/>"
    "<circle cx='32' cy='32' r='9' fill='white'/></svg>";

void draw_scene(SkCanvas* canvas, const Fonts& fonts, const sk_sp<SkTextBlob>& shaped,
                const sk_sp<SkSVGDOM>& svg) {
  canvas->clear(kBackground);

  // A card: a blurred shadow under a rounded rectangle filled with a linear gradient.
  const SkRRect card = SkRRect::MakeRectXY(SkRect::MakeXYWH(40, 36, 260, 150), 18, 18);
  SkPaint shadow;
  shadow.setAntiAlias(true);
  shadow.setColor(0x66000000);
  shadow.setMaskFilter(SkMaskFilter::MakeBlur(kNormal_SkBlurStyle, 8.0f));
  canvas->drawRRect(card.makeOffset(0, 8), shadow);

  const SkPoint ends[2] = {{40, 36}, {300, 186}};
  const SkColor4f colors[2] = {SkColor4f::FromColor(0xFF5B8DEF), SkColor4f::FromColor(0xFF9B5BEF)};
  SkPaint fill;
  fill.setAntiAlias(true);
  fill.setShader(SkShaders::LinearGradient(
      ends, SkGradient(SkGradient::Colors({colors, 2}, SkTileMode::kClamp), {})));
  canvas->drawRRect(card, fill);

  // Text in the variable font, at weight 400 and 700.
  SkPaint ink;
  ink.setAntiAlias(true);
  ink.setColor(SK_ColorWHITE);
  SkFont regular(fonts.regular, 22);
  regular.setEdging(SkFont::Edging::kAntiAlias);
  SkFont bold(fonts.bold, 22);
  bold.setEdging(SkFont::Edging::kAntiAlias);
  canvas->drawString("Bloom on Skia", 64, 92, bold, ink);
  canvas->drawString("Regular 400, bold 700", 64, 126, regular, ink);

  // A line shaped by HarfBuzz (SkShaper), its breaks from ICU (SkUnicode).
  ink.setColor(0xFF1B1B24);
  canvas->drawTextBlob(shaped, 40, 220, ink);

  // The SVG module (Expat parses it) and the drawable.
  canvas->save();
  canvas->translate(340, 60);
  svg->render(canvas);
  canvas->restore();
  canvas->save();
  canvas->translate(400, 170);
  Badge badge;
  canvas->drawDrawable(&badge);
  canvas->restore();
}

bool write_png(const std::string& path, const SkPixmap& pixels) {
  SkFILEWStream file(path.c_str());
  if (!file.isValid() || !SkPngEncoder::Encode(&file, pixels, {})) return fail(("cannot write " + path).c_str());
  std::printf("wrote %s\n", path.c_str());
  return true;
}

// The raster result must show each part of the scene.
bool check_raster(const SkPixmap& pixels) {
  auto at = [&](int x, int y) { return *pixels.addr32(x, y); };
  auto is_background = [&](int x, int y) { return at(x, y) == SkPreMultiplyColor(kBackground); };
  bool ok = true;
  if (is_background(170, 160)) ok = fail("the card is not drawn");
  if (is_background(170, 192) || at(170, 192) == at(170, 160)) ok = fail("the card's shadow is not drawn");
  if (at(50, 170) == at(290, 50)) ok = fail("the gradient does not vary");
  if (is_background(372, 92)) ok = fail("the SVG is not drawn");
  if (is_background(412, 182)) ok = fail("the drawable is not drawn");
  int shaped = 0;
  for (int y = 200; y < 230; ++y)
    for (int x = 40; x < 440; ++x) shaped += !is_background(x, y);
  if (shaped < 200) ok = fail("the shaped line is not drawn");
  return ok;
}

// ---- Vulkan --------------------------------------------------------------------------------------

struct Vulkan {
  void* library = nullptr;
  PFN_vkGetInstanceProcAddr getInstanceProcAddr = nullptr;
  PFN_vkGetDeviceProcAddr getDeviceProcAddr = nullptr;
  VkInstance instance = VK_NULL_HANDLE;
  VkPhysicalDevice physicalDevice = VK_NULL_HANDLE;
  VkDevice device = VK_NULL_HANDLE;
  VkQueue queue = VK_NULL_HANDLE;
  uint32_t queueFamily = 0;
  uint32_t apiVersion = VK_API_VERSION_1_1;
  std::vector<const char*> instanceExtensions, deviceExtensions;
  VkPhysicalDeviceFeatures2 features{VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2};
  skgpu::VulkanPreferredFeatures skiaFeatures;  // owns structs chained to `features`
  skgpu::VulkanExtensions extensions;
  std::string deviceName;

  template <typename F> F instanceProc(const char* name) const {
    return reinterpret_cast<F>(getInstanceProcAddr(instance, name));
  }

  ~Vulkan() {
    if (device) {
      instanceProc<PFN_vkDeviceWaitIdle>("vkDeviceWaitIdle")(device);
      instanceProc<PFN_vkDestroyDevice>("vkDestroyDevice")(device, nullptr);
    }
    if (instance) instanceProc<PFN_vkDestroyInstance>("vkDestroyInstance")(instance, nullptr);
    if (library) dlclose(library);
  }

  // Whether the loader finds a driver with a device: asked of a bare instance first, so Skia's
  // feature flow below only starts where it can finish.
  bool any_device(std::string& why) {
    VkApplicationInfo application{VK_STRUCTURE_TYPE_APPLICATION_INFO};
    application.apiVersion = apiVersion;
    VkInstanceCreateInfo info{VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
    info.pApplicationInfo = &application;
    VkInstance probe = VK_NULL_HANDLE;
    const VkResult result = instanceProc<PFN_vkCreateInstance>("vkCreateInstance")(&info, nullptr, &probe);
    if (result != VK_SUCCESS) return why = "no Vulkan driver (vkCreateInstance: " + std::to_string(result) + ")", false;
    uint32_t count = 0;
    reinterpret_cast<PFN_vkEnumeratePhysicalDevices>(getInstanceProcAddr(probe, "vkEnumeratePhysicalDevices"))(
        probe, &count, nullptr);
    reinterpret_cast<PFN_vkDestroyInstance>(getInstanceProcAddr(probe, "vkDestroyInstance"))(probe, nullptr);
    if (!count) return why = "no Vulkan device", false;
    return true;
  }

  // An instance and a device with one graphics queue, with what Skia asks for added (its
  // VulkanPreferredFeatures flow). False, with the reason in `why`, when there is no loader or device.
  bool open(std::string& why) {
    library = dlopen("libvulkan.so.1", RTLD_NOW | RTLD_LOCAL);
    if (!library) return why = "no Vulkan loader (libvulkan.so.1)", false;
    getInstanceProcAddr =
        reinterpret_cast<PFN_vkGetInstanceProcAddr>(dlsym(library, "vkGetInstanceProcAddr"));
    if (!getInstanceProcAddr) return why = "the loader has no vkGetInstanceProcAddr", false;

    if (auto enumerateVersion = instanceProc<PFN_vkEnumerateInstanceVersion>("vkEnumerateInstanceVersion")) {
      uint32_t loader = 0;
      if (enumerateVersion(&loader) == VK_SUCCESS) apiVersion = std::min(loader, uint32_t(VK_API_VERSION_1_3));
    }
    if (apiVersion < VK_API_VERSION_1_1) return why = "the loader is older than Vulkan 1.1", false;
    if (!any_device(why)) return false;
    skiaFeatures.init(apiVersion);

    auto enumerateInstanceExtensions =
        instanceProc<PFN_vkEnumerateInstanceExtensionProperties>("vkEnumerateInstanceExtensionProperties");
    uint32_t count = 0;
    enumerateInstanceExtensions(nullptr, &count, nullptr);
    std::vector<VkExtensionProperties> available(count);
    enumerateInstanceExtensions(nullptr, &count, available.data());
    skiaFeatures.addToInstanceExtensions(available.data(), available.size(), instanceExtensions);

    VkApplicationInfo application{VK_STRUCTURE_TYPE_APPLICATION_INFO};
    application.pApplicationName = "oxyos-skia-check";
    application.apiVersion = apiVersion;
    VkInstanceCreateInfo instanceInfo{VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
    instanceInfo.pApplicationInfo = &application;
    instanceInfo.enabledExtensionCount = uint32_t(instanceExtensions.size());
    instanceInfo.ppEnabledExtensionNames = instanceExtensions.data();
    VkResult result = instanceProc<PFN_vkCreateInstance>("vkCreateInstance")(&instanceInfo, nullptr, &instance);
    if (result != VK_SUCCESS) return why = "no Vulkan driver (vkCreateInstance: " + std::to_string(result) + ")", false;
    getDeviceProcAddr = instanceProc<PFN_vkGetDeviceProcAddr>("vkGetDeviceProcAddr");

    auto enumerateDevices = instanceProc<PFN_vkEnumeratePhysicalDevices>("vkEnumeratePhysicalDevices");
    count = 0;
    enumerateDevices(instance, &count, nullptr);
    std::vector<VkPhysicalDevice> devices(count);
    enumerateDevices(instance, &count, devices.data());
    auto queueFamilies = instanceProc<PFN_vkGetPhysicalDeviceQueueFamilyProperties>(
        "vkGetPhysicalDeviceQueueFamilyProperties");
    for (VkPhysicalDevice candidate : devices) {
      uint32_t families = 0;
      queueFamilies(candidate, &families, nullptr);
      std::vector<VkQueueFamilyProperties> properties(families);
      queueFamilies(candidate, &families, properties.data());
      for (uint32_t i = 0; i < families && !physicalDevice; ++i) {
        if (properties[i].queueFlags & VK_QUEUE_GRAPHICS_BIT) physicalDevice = candidate, queueFamily = i;
      }
      if (physicalDevice) break;
    }
    if (!physicalDevice) return why = "no Vulkan device with a graphics queue", false;
    VkPhysicalDeviceProperties properties;
    instanceProc<PFN_vkGetPhysicalDeviceProperties>("vkGetPhysicalDeviceProperties")(physicalDevice, &properties);
    deviceName = properties.deviceName;
    // The device's own version caps what Skia may use.
    apiVersion = std::min(apiVersion, properties.apiVersion);

    auto enumerateDeviceExtensions =
        instanceProc<PFN_vkEnumerateDeviceExtensionProperties>("vkEnumerateDeviceExtensionProperties");
    count = 0;
    enumerateDeviceExtensions(physicalDevice, nullptr, &count, nullptr);
    std::vector<VkExtensionProperties> deviceAvailable(count);
    enumerateDeviceExtensions(physicalDevice, nullptr, &count, deviceAvailable.data());
    skiaFeatures.addFeaturesToQuery(deviceAvailable.data(), deviceAvailable.size(), features);
    instanceProc<PFN_vkGetPhysicalDeviceFeatures2>("vkGetPhysicalDeviceFeatures2")(physicalDevice, &features);
    skiaFeatures.addFeaturesToEnable(deviceExtensions, features);

    const float priority = 1.0f;
    VkDeviceQueueCreateInfo queueInfo{VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO};
    queueInfo.queueFamilyIndex = queueFamily;
    queueInfo.queueCount = 1;
    queueInfo.pQueuePriorities = &priority;
    VkDeviceCreateInfo deviceInfo{VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO};
    deviceInfo.pNext = &features;
    deviceInfo.queueCreateInfoCount = 1;
    deviceInfo.pQueueCreateInfos = &queueInfo;
    deviceInfo.enabledExtensionCount = uint32_t(deviceExtensions.size());
    deviceInfo.ppEnabledExtensionNames = deviceExtensions.data();
    result = instanceProc<PFN_vkCreateDevice>("vkCreateDevice")(physicalDevice, &deviceInfo, nullptr, &device);
    if (result != VK_SUCCESS) return why = "vkCreateDevice: " + std::to_string(result), false;
    reinterpret_cast<PFN_vkGetDeviceQueue>(getDeviceProcAddr(device, "vkGetDeviceQueue"))(device, queueFamily, 0, &queue);
    return true;
  }

  skgpu::VulkanGetProc getProc() const {
    return [this](const char* name, VkInstance inst, VkDevice dev) -> PFN_vkVoidFunction {
      if (dev != VK_NULL_HANDLE) return getDeviceProcAddr(dev, name);
      return getInstanceProcAddr(inst, name);
    };
  }

  skgpu::VulkanBackendContext backendContext() {
    skgpu::VulkanBackendContext context;
    context.fInstance = instance;
    context.fPhysicalDevice = physicalDevice;
    context.fDevice = device;
    context.fQueue = queue;
    context.fGraphicsQueueIndex = queueFamily;
    context.fMaxAPIVersion = apiVersion;
    context.fGetProc = getProc();
    extensions.init(context.fGetProc, instance, physicalDevice, uint32_t(instanceExtensions.size()),
                    instanceExtensions.data(), uint32_t(deviceExtensions.size()), deviceExtensions.data());
    context.fVkExtensions = &extensions;
    context.fDeviceFeatures2 = &features;
    context.fMemoryAllocator = skgpu::VulkanMemoryAllocators::Make(context, skgpu::ThreadSafe::kNo);
    return context;
  }
};

}  // namespace

int main(int argc, char** argv) {
  if (argc != 3) {
    std::fprintf(stderr, "usage: oxyos-skia-check <variable-font.ttf> <out-dir>\n");
    return 2;
  }
  const std::string out = argv[2];

  // Fonts: the empty custom manager, which knows only the files it is given (no fontconfig).
  Fonts fonts;
  fonts.manager = SkFontMgr_New_Custom_Empty();
  fonts.regular = at_weight(*fonts.manager, argv[1], 400);
  fonts.bold = at_weight(*fonts.manager, argv[1], 700);
  if (!fonts.regular || !fonts.bold) return fail("the font cannot be loaded"), 1;
  sk_sp<SkTypeface> plain = fonts.manager->makeFromFile(argv[1]);
  if (!plain) return fail("makeFromFile cannot load the font"), 1;
  SkFontArguments::VariationPosition::Coordinate position[8];
  const int axes = fonts.bold->getVariationDesignPosition(position);
  const auto wght = std::find_if(position, position + std::max(axes, 0),
                                 [](const auto& c) { return c.axis == SkSetFourByteTag('w', 'g', 'h', 't'); });
  if (wght == position + std::max(axes, 0) || wght->value != 700) return fail("the 'wght' axis is not set"), 1;
  // Unhinted, unrounded advances, so the axis shows in the width.
  auto width = [](const sk_sp<SkTypeface>& typeface) {
    SkFont font(typeface, 22);
    font.setSubpixel(true);
    font.setLinearMetrics(true);
    font.setHinting(SkFontHinting::kNone);
    return font.measureText("Weight", 6, SkTextEncoding::kUTF8);
  };
  const float regularWidth = width(fonts.regular), boldWidth = width(fonts.bold);
  std::printf("font: %d axes, \"Weight\" at 22 px is %.2f px wide at wght 400, %.2f at 700\n", axes,
              regularWidth, boldWidth);
  if (!(boldWidth > regularWidth)) return fail("wght 700 is not wider than 400"), 1;

  // A line shaped by HarfBuzz (kerning, a combining accent placed over its base), with ICU's
  // breaks and bidi.
  const char line[] = "Shaped by HarfBuzz: AVATAR, Te\xCC\x81l\xC3\xA9phone";
  auto shaper = SkShapers::HB::ShaperDrivenWrapper(SkUnicodes::ICU::Make(), fonts.manager);
  if (!shaper) return fail("SkShaper has no HarfBuzz shaper"), 1;
  SkTextBlobBuilderRunHandler handler(line, {0, 0});
  shaper->shape(line, std::strlen(line), SkFont(fonts.regular, 18), true, 1000, &handler);
  sk_sp<SkTextBlob> shaped = handler.makeBlob();
  if (!shaped) return fail("the shaper produced no text"), 1;

  SkMemoryStream svgStream(kSvg, sizeof kSvg - 1, false);
  sk_sp<SkSVGDOM> svg = SkSVGDOM::MakeFromStream(svgStream);
  if (!svg) return fail("the SVG does not parse"), 1;

  // Raster: Skia draws into pixels the program owns, BGRA premultiplied, as the host's canvases are.
  const SkImageInfo info = SkImageInfo::Make(kWidth, kHeight, kBGRA_8888_SkColorType, kPremul_SkAlphaType);
  std::vector<uint32_t> cpu(size_t(kWidth) * kHeight);
  sk_sp<SkSurface> raster = SkSurfaces::WrapPixels(info, cpu.data(), info.minRowBytes());
  if (!raster) return fail("SkSurfaces::WrapPixels failed"), 1;
  draw_scene(raster->getCanvas(), fonts, shaped, svg);
  const SkPixmap cpuPixels(info, cpu.data(), info.minRowBytes());
  if (!write_png(out + "/raster.png", cpuPixels)) return 1;
  if (!check_raster(cpuPixels)) return 1;
  std::printf("raster: drawn and checked\n");

  // Vulkan: the same scene on Ganesh, read back.
  Vulkan vulkan;
  std::string why;
  if (!vulkan.open(why)) {
    std::printf("vulkan: skipped, %s\n", why.c_str());
    return 0;
  }
  std::printf("vulkan: %s, API %u.%u\n", vulkan.deviceName.c_str(), VK_API_VERSION_MAJOR(vulkan.apiVersion),
              VK_API_VERSION_MINOR(vulkan.apiVersion));
  skgpu::VulkanBackendContext backend = vulkan.backendContext();
  if (!backend.fMemoryAllocator) return fail("no Vulkan memory allocator"), 1;
  sk_sp<GrDirectContext> context = GrDirectContexts::MakeVulkan(backend);
  if (!context) return fail("GrDirectContexts::MakeVulkan failed"), 1;
  std::vector<uint32_t> gpu(cpu.size());
  {
    sk_sp<SkSurface> surface =
        SkSurfaces::RenderTarget(context.get(), skgpu::Budgeted::kNo, info, 0, kTopLeft_GrSurfaceOrigin, nullptr);
    if (!surface) return fail("SkSurfaces::RenderTarget failed"), 1;
    draw_scene(surface->getCanvas(), fonts, shaped, svg);
    context->flushAndSubmit(surface.get(), GrSyncCpu::kYes);
    if (!surface->readPixels(info, gpu.data(), info.minRowBytes(), 0, 0)) return fail("readPixels failed"), 1;
  }
  context->flushAndSubmit(GrSyncCpu::kYes);
  context.reset();
  const SkPixmap gpuPixels(info, gpu.data(), info.minRowBytes());
  if (!write_png(out + "/vulkan.png", gpuPixels)) return 1;

  // Ganesh and the raster backend antialias, blur and dither differently; a missing part of the scene
  // is a large difference over many pixels.
  int maxDiff = 0;
  size_t differing = 0, near = 0, far = 0;
  for (size_t i = 0; i < cpu.size(); ++i) {
    int pixelMax = 0;
    for (int shift = 0; shift < 32; shift += 8)
      pixelMax = std::max(pixelMax, std::abs(int((cpu[i] >> shift) & 0xFF) - int((gpu[i] >> shift) & 0xFF)));
    maxDiff = std::max(maxDiff, pixelMax);
    differing += pixelMax > 0;
    near += pixelMax > 2;
    far += pixelMax > 64;
  }
  std::printf("vulkan vs raster: max channel difference %d; %zu pixels differ (%.2f %%), %zu by more than 2,"
              " %zu by more than 64\n", maxDiff, differing, 100.0 * differing / cpu.size(), near, far);
  if (!check_raster(gpuPixels)) return 1;
  if (far > cpu.size() / 200) return fail("Vulkan's drawing is far from the raster one"), 1;
  return 0;
}
