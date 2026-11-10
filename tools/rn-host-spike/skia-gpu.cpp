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

#include "frame-diagnostics.h"
#include "pipeline-cache.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cstring>
#include <filesystem>
#include <iostream>

namespace oxyos {

namespace {

// The driver's vkCreateGraphicsPipelines, and Ganesh's calls to it timed and counted for the
// frame's diagnostics: with VK_EXT_pipeline_creation_cache_control (oxyos-paint enables it where
// the device has it) Ganesh asks the driver's cache first (FAIL_ON_PIPELINE_COMPILE_REQUIRED), so a
// pipeline made without that flag was compiled; without it, every pipeline counts as compiled.
PFN_vkCreateGraphicsPipelines driverCreateGraphicsPipelines = nullptr;

VKAPI_ATTR VkResult VKAPI_CALL countedCreateGraphicsPipelines(VkDevice device, VkPipelineCache cache, uint32_t count,
                                                              const VkGraphicsPipelineCreateInfo* infos,
                                                              const VkAllocationCallbacks* allocator,
                                                              VkPipeline* pipelines) {
  auto started = std::chrono::steady_clock::now();
  VkResult result = driverCreateGraphicsPipelines(device, cache, count, infos, allocator, pipelines);
  double ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - started).count();
  bool lookup = count > 0 && (infos[0].flags & VK_PIPELINE_CREATE_FAIL_ON_PIPELINE_COMPILE_REQUIRED_BIT_EXT) != 0;
  FrameDiagnostics::get().pipelines(result == VK_SUCCESS ? static_cast<int>(count) : 0,
                                    result == VK_SUCCESS && !lookup ? static_cast<int>(count) : 0, ms);
  return result;
}

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
  // Set once the context is made (on the painter's start thread); from then on it is the loop's.
  std::atomic<bool> made{false};
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
    if (on != VK_NULL_HANDLE) {
      auto proc = made.getDeviceProcAddr(on, name);
      if (proc && std::strcmp(name, "vkCreateGraphicsPipelines") == 0) {
        driverCreateGraphicsPipelines = reinterpret_cast<PFN_vkCreateGraphicsPipelines>(proc);
        return reinterpret_cast<PFN_vkVoidFunction>(&countedCreateGraphicsPipelines);
      }
      return proc;
    }
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
    // is its own, and its pipelineCacheUUID changes with its build) and by Skia's revision (what
    // its programs' keys mean). Every other directory beside it is removed: a driver or Skia
    // updated leaves its old cache behind once.
    auto properties = reinterpret_cast<PFN_vkGetPhysicalDeviceProperties>(
        made.getInstanceProcAddr(instance, "vkGetPhysicalDeviceProperties"));
    if (properties) {
      VkPhysicalDeviceProperties device{};
      properties(physical, &device);
      std::string uuid;
      for (uint8_t byte : device.pipelineCacheUUID) {
        char hex[3];
        std::snprintf(hex, sizeof hex, "%02x", byte);
        uuid += hex;
      }
      char name[160];
      std::snprintf(name, sizeof name, "%04x-%04x-%08x-%s-%.12s", device.vendorID, device.deviceID,
                    device.driverVersion, uuid.c_str(), OXYOS_SKIA_REVISION);
      made.cache = std::make_unique<PipelineCache>(std::filesystem::path(pipelineCacheDirectory()) / name,
                                                    [] { FrameDiagnostics::get().shaderCompiled(); });
      made.cache->removeOthers();
      options.fPersistentCache = made.cache.get();
    }
  }
  made.direct = GrDirectContexts::MakeVulkan(backend, options);
  made.made = made.direct != nullptr;
  return made.made;
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

void storePipelineCache() {
  Context& made = context();
  if (!made.made || !made.cache) return;
  if (made.direct->hasNewVkPipelineCacheData()) made.direct->storeVkPipelineCacheData(PipelineCache::kMaxBytes / 2);
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
