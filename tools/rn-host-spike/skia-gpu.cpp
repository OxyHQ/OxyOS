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

#include <iostream>

namespace oxyos {

namespace {

struct Context {
  OxyOSPaint* paint = nullptr;
  OxyOSPaintVulkan vulkan{};
  PFN_vkGetInstanceProcAddr getInstanceProcAddr = nullptr;
  PFN_vkGetDeviceProcAddr getDeviceProcAddr = nullptr;
  skgpu::VulkanExtensions extensions;
  sk_sp<GrDirectContext> direct;
};

Context& context() {
  // Kept for the process, as the painter is: never destroyed, so no canvas outlives it.
  static Context* made = new Context;
  return *made;
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
    return made.getInstanceProcAddr(at, name);
  };
  made.extensions.init(backend.fGetProc, instance, physical, static_cast<uint32_t>(made.vulkan.instance_extension_count),
                       made.vulkan.instance_extensions, static_cast<uint32_t>(made.vulkan.device_extension_count),
                       made.vulkan.device_extensions);
  backend.fVkExtensions = &made.extensions;
  backend.fDeviceFeatures2 = static_cast<const VkPhysicalDeviceFeatures2*>(made.vulkan.features);
  backend.fMemoryAllocator = skgpu::VulkanMemoryAllocators::Make(backend, skgpu::ThreadSafe::kNo);
  if (!backend.fMemoryAllocator) return false;
  made.direct = GrDirectContexts::MakeVulkan(backend);
  return made.direct != nullptr;
}

}  // namespace

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
