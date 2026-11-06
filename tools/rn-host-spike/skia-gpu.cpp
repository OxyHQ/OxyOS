#include "skia-gpu.h"

#include <include/gpu/MutableTextureState.h>
#include <include/gpu/ganesh/GrBackendSurface.h>
#include <include/gpu/ganesh/GrContextOptions.h>
#include <include/gpu/ganesh/GrDirectContext.h>
#include <include/gpu/ganesh/SkSurfaceGanesh.h>
#include <include/gpu/ganesh/vk/GrVkBackendSurface.h>
#include <include/gpu/ganesh/vk/GrVkDirectContext.h>
#include <include/gpu/ganesh/vk/GrVkTypes.h>
#include <include/gpu/vk/VulkanBackendContext.h>
#include <include/gpu/vk/VulkanExtensions.h>
#include <include/gpu/vk/VulkanMutableTextureState.h>
// Skia's Vulkan Memory Allocator behind its VulkanMemoryAllocator interface: a Ganesh context
// needs one, and Skia's own tools make it this way.
#include <src/gpu/GpuTypesPriv.h>
#include <src/gpu/vk/vulkanmemoryallocator/VulkanMemoryAllocatorPriv.h>

#include <include/core/SkData.h>

#include <algorithm>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <map>
#include <sstream>

namespace oxyos {

namespace {

// A file of the cache: this, the key's length (4 bytes), the key, then the data. A key is found by
// its hash and checked whole, so two keys never share a file's data.
constexpr char kMagic[8] = {'O', 'X', 'S', 'K', 'C', 'A', 'C', '1'};

uint64_t hashOf(const SkData& key) {
  // FNV-1a.
  uint64_t hash = 0xcbf29ce484222325ull;
  for (size_t i = 0; i < key.size(); ++i) hash = (hash ^ key.bytes()[i]) * 0x100000001b3ull;
  return hash;
}

class PipelineCache final : public GrContextOptions::PersistentCache {
 public:
  explicit PipelineCache(std::filesystem::path directory) : directory_(std::move(directory)) {}

  sk_sp<SkData> load(const SkData& key) override {
    std::string wanted(reinterpret_cast<const char*>(key.data()), key.size());
    if (auto it = pending_.find(wanted); it != pending_.end()) return it->second;
    std::ifstream in(fileOf(key), std::ios::binary);
    if (!in) return nullptr;
    std::stringstream read;
    read << in.rdbuf();
    std::string bytes = read.str();
    uint32_t length = 0;
    if (bytes.size() < sizeof kMagic + sizeof length || std::memcmp(bytes.data(), kMagic, sizeof kMagic) != 0) {
      return nullptr;
    }
    std::memcpy(&length, bytes.data() + sizeof kMagic, sizeof length);
    size_t at = sizeof kMagic + sizeof length;
    if (bytes.size() < at + length || bytes.compare(at, length, wanted) != 0) return nullptr;
    return SkData::MakeWithCopy(bytes.data() + at + length, bytes.size() - at - length);
  }

  void store(const SkData& key, const SkData& data, const SkString&) override {
    pending_[std::string(reinterpret_cast<const char*>(key.data()), key.size())] =
        SkData::MakeWithCopy(data.data(), data.size());
  }

  // Writes what is pending, each file whole or not at all (written aside, then renamed: the
  // application's other processes read either the file before or this one).
  void write() {
    if (pending_.empty()) return;
    std::error_code error;
    std::filesystem::create_directories(directory_, error);
    for (const auto& [key, data] : pending_) {
      auto keyData = SkData::MakeWithoutCopy(key.data(), key.size());
      auto path = fileOf(*keyData);
      auto aside = path;
      aside += ".new";
      {
        std::ofstream out(aside, std::ios::binary | std::ios::trunc);
        uint32_t length = static_cast<uint32_t>(key.size());
        out.write(kMagic, sizeof kMagic);
        out.write(reinterpret_cast<const char*>(&length), sizeof length);
        out.write(key.data(), static_cast<std::streamsize>(key.size()));
        out.write(static_cast<const char*>(data->data()), static_cast<std::streamsize>(data->size()));
        if (!out) {
          std::filesystem::remove(aside, error);
          continue;
        }
      }
      std::filesystem::rename(aside, path, error);
      if (error) std::filesystem::remove(aside, error);
    }
    pending_.clear();
  }

 private:
  std::filesystem::path fileOf(const SkData& key) const {
    char name[32];
    std::snprintf(name, sizeof name, "%016llx.bin", static_cast<unsigned long long>(hashOf(key)));
    return directory_ / name;
  }

  std::filesystem::path directory_;
  std::map<std::string, sk_sp<SkData>> pending_;
};

std::string& pipelineCacheDirectory() {
  static std::string directory;
  return directory;
}

struct Context {
  OxyOSPaint* paint = nullptr;
  OxyOSPaintVulkan vulkan{};
  PFN_vkGetInstanceProcAddr getInstanceProcAddr = nullptr;
  PFN_vkGetDeviceProcAddr getDeviceProcAddr = nullptr;
  skgpu::VulkanExtensions extensions;
  std::unique_ptr<PipelineCache> cache;
  sk_sp<GrDirectContext> direct;
};

Context& context() {
  // Kept for the process, as the painter is: never destroyed, so no canvas outlives it.
  static Context* made = new Context;
  return *made;
}

// Skia asks the loader for every instance extension, to read their spec versions; the loader then
// opens every Vulkan driver installed, another GPU's too. The painter's instance has its GPU's
// drivers alone (docs/adr/0031-gpu-buffers.mdx, decision 14) and Skia is told of no instance
// extension, so it is answered with none and no driver is opened.
VKAPI_ATTR VkResult VKAPI_CALL noInstanceExtensions(const char*, uint32_t* count, VkExtensionProperties*) {
  *count = 0;
  return VK_SUCCESS;
}

bool make(Context& made, OxyOSPaint* paint) {
  made.paint = paint;
  if (oxyos_paint_vulkan(paint, &made.vulkan) != 0) {
    std::cerr << "[skia] the painter's Vulkan objects: " << oxyos_last_error() << "\n";
    return false;
  }
  auto instance = reinterpret_cast<VkInstance>(made.vulkan.instance);
  auto physical = reinterpret_cast<VkPhysicalDevice>(made.vulkan.physical_device);
  auto device = reinterpret_cast<VkDevice>(made.vulkan.device);
  made.getInstanceProcAddr = reinterpret_cast<PFN_vkGetInstanceProcAddr>(made.vulkan.get_instance_proc_addr);
  made.getDeviceProcAddr =
      reinterpret_cast<PFN_vkGetDeviceProcAddr>(made.getInstanceProcAddr(instance, "vkGetDeviceProcAddr"));
  if (!made.getDeviceProcAddr) return false;
  skgpu::VulkanBackendContext backend;
  backend.fInstance = instance;
  backend.fPhysicalDevice = physical;
  backend.fDevice = device;
  backend.fQueue = reinterpret_cast<VkQueue>(made.vulkan.queue);
  backend.fGraphicsQueueIndex = made.vulkan.queue_family;
  backend.fMaxAPIVersion = made.vulkan.api_version;
  backend.fGetProc = [&made](const char* name, VkInstance at, VkDevice on) -> PFN_vkVoidFunction {
    if (on != VK_NULL_HANDLE) return made.getDeviceProcAddr(on, name);
    if (at == VK_NULL_HANDLE && std::strcmp(name, "vkEnumerateInstanceExtensionProperties") == 0)
      return reinterpret_cast<PFN_vkVoidFunction>(&noInstanceExtensions);
    return made.getInstanceProcAddr(at, name);
  };
  made.extensions.init(backend.fGetProc, instance, physical, static_cast<uint32_t>(made.vulkan.instance_extension_count),
                       made.vulkan.instance_extensions, static_cast<uint32_t>(made.vulkan.device_extension_count),
                       made.vulkan.device_extensions);
  backend.fVkExtensions = &made.extensions;
  backend.fDeviceFeatures2 = static_cast<const VkPhysicalDeviceFeatures2*>(made.vulkan.features);
  backend.fMemoryAllocator = skgpu::VulkanMemoryAllocators::Make(backend, skgpu::ThreadSafe::kNo);
  if (!backend.fMemoryAllocator) return false;
  GrContextOptions options;
  // The glyph atlases' pages at 2 MiB each (A8 2048 x 1024, colour 1024 x 512) instead of Skia's 4 and
  // 8 MiB: a page holds the glyphs of a screen of text at a desktop's sizes, and Ganesh adds pages
  // (four at most) as more are drawn and drops them when unused.
  options.fGlyphCacheTextureMaximumBytes = 2 << 20;
  if (!pipelineCacheDirectory().empty()) {
    // By the GPU and its driver (the SPIR-V follows the device's capabilities; the driver's cache
    // is its own) and by Skia's revision (what its programs' keys mean).
    auto properties = reinterpret_cast<PFN_vkGetPhysicalDeviceProperties>(
        made.getInstanceProcAddr(instance, "vkGetPhysicalDeviceProperties"));
    if (properties) {
      VkPhysicalDeviceProperties device{};
      properties(physical, &device);
      char name[96];
      std::snprintf(name, sizeof name, "%04x-%04x-%08x-%s", device.vendorID, device.deviceID, device.driverVersion,
                    OXYOS_SKIA_REVISION);
      made.cache = std::make_unique<PipelineCache>(std::filesystem::path(pipelineCacheDirectory()) / name);
      options.fPersistentCache = made.cache.get();
    }
  }
  made.direct = GrDirectContexts::MakeVulkan(backend, options);
  return made.direct != nullptr;
}

}  // namespace

void fitResourceCache(GrDirectContext* direct, int width, int height) {
  static size_t largest = 0;
  size_t bytes = static_cast<size_t>(std::max(width, 0)) * static_cast<size_t>(std::max(height, 0)) * 4;
  if (!direct || bytes <= largest) return;
  largest = bytes;
  direct->setResourceCacheLimit(std::max<size_t>(16 << 20, 3 * largest));
}

void setPipelineCacheDirectory(std::string directory) { pipelineCacheDirectory() = std::move(directory); }

void storePipelineCache(GrDirectContext* direct) {
  Context& made = context();
  if (!made.cache || direct != made.direct.get()) return;
  if (direct->hasNewVkPipelineCacheData()) direct->storeVkPipelineCacheData();
  made.cache->write();
}

GrDirectContext* skiaContext(OxyOSPaint* paint) {
  if (!paint) return nullptr;
  Context& made = context();
  if (made.paint == paint) return made.direct.get();
  if (made.paint) {
    // One painter per process (window.h): another one would be a second device.
    std::cerr << "[skia] a second painter is not drawn on\n";
    return nullptr;
  }
  if (!make(made, paint)) {
    std::cerr << "[skia] Ganesh cannot draw on " << oxyos_paint_device_name(paint) << ": drawing on the CPU\n";
    made.direct.reset();
  }
  return made.direct.get();
}

int64_t presentGpu(OxyOSPaint* paint, OxyOSClient* client, uint32_t surface, uint64_t configure, const Canvas& canvas,
                   uint32_t format, const std::vector<OxyOSRect>& damage) {
  GrDirectContext* direct = skiaContext(paint);
  if (!direct || !canvas.gpu) return -1;
  SkSurface* drawn = canvas.gpu->surface.get();
  GrBackendRenderTarget target = SkSurfaces::GetBackendRenderTarget(drawn, SkSurfaces::BackendHandleAccess::kFlushRead);
  GrVkImageInfo image;
  if (!GrBackendRenderTargets::GetVkImageInfo(target, &image)) return -1;
  // The frame's drawing submitted to the painter's queue, the image left ready to be copied from,
  // on that queue's family; oxyos-paint's copy follows it on the same queue.
  skgpu::MutableTextureState readable =
      skgpu::MutableTextureStates::MakeVulkan(VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, context().vulkan.queue_family);
  direct->flush(drawn, GrFlushInfo(), &readable);
  if (!direct->submit(GrSyncCpu::kNo)) return -1;
  return oxyos_paint_present(paint, client, surface, configure, reinterpret_cast<uint64_t>(image.fImage),
                             static_cast<uint32_t>(canvas.width), static_cast<uint32_t>(canvas.height), format,
                             damage.data(), damage.size());
}

}  // namespace oxyos
