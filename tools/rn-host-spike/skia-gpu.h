// Skia on the GPU (docs/adr/0031-gpu-buffers.mdx, decision 12): Ganesh on the Vulkan device
// oxyos-paint opened (the one the compositor shares buffers with), its frames handed to oxyos-paint,
// which copies what changed into the surface's exported DMA-BUF images and presents them.
#pragma once

#include "renderer.h"

#include <oxyos_client.h>

#include <include/core/SkSurface.h>

#include <string>

class GrDirectContext;

namespace oxyos {

// A canvas drawn on the GPU: a Ganesh surface (B8G8R8A8, premultiplied) on the painter's device.
struct GpuCanvas {
  sk_sp<SkSurface> surface;
};

// The Ganesh context on `paint`'s device, made the first time it is asked for and kept as long as
// the process: the painter is never freed once set. Null for no painter, or a device Skia cannot
// draw on (said once).
GrDirectContext* skiaContext(OxyOSPaint* paint);

// Ganesh's persistent cache (GrContextOptions::fPersistentCache), as Chrome's and Flutter's: the
// SPIR-V of every pipeline it compiled and the driver's Vulkan pipeline cache, kept in `directory`
// (the application's cache directory's `skia/`), by its GPU (vendor, device, driver) and Skia's
// revision, so that a pipeline is compiled once, not at every start, nor on the first draw of
// each thing after it. Set before the context is made (Window starts it on a thread of its own);
// none, nothing is kept. What Ganesh gives it is held in memory until storePipelineCache writes
// it, so no file is written while a frame is drawn.
void setPipelineCacheDirectory(std::string directory);
// Writes what Ganesh learned since the last time (memory.h, once the window is idle): each new
// pipeline's shaders, and the driver's pipeline cache when it changed.
void storePipelineCache(GrDirectContext* context);

// Bounds Skia's GPU resource cache (pictures uploaded, offscreen layers, glyph atlases; its default
// is 256 MiB) to the largest canvas drawn so far: three of its size, at least 16 MiB. A resource
// past it goes once nothing holds it, the least recently used first. Called as a canvas is made.
void fitResourceCache(GrDirectContext* context, int width, int height);

// Presents a canvas drawn on the GPU in `surface`'s GPU buffers: its drawing submitted, its image
// made ready to be copied from (TRANSFER_SRC_OPTIMAL), and oxyos_paint_present's sequence returned
// (negative on failure, as oxyos_paint_present).
int64_t presentGpu(OxyOSPaint* paint, OxyOSClient* client, uint32_t surface, uint64_t configure,
                   const Canvas& canvas, uint32_t format, const std::vector<OxyOSRect>& damage);

}  // namespace oxyos
