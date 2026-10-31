// Skia on the GPU (docs/adr/0031-gpu-buffers.mdx, decision 12): Ganesh on the Vulkan device
// oxyos-paint opened (the one the compositor shares buffers with), its frames handed to oxyos-paint,
// which copies what changed into the surface's exported DMA-BUF images and presents them.
#pragma once

#include "renderer.h"

#include <oxyos_client.h>

#include <include/core/SkSurface.h>

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

// Presents a canvas drawn on the GPU in `surface`'s GPU buffers: its drawing submitted, its image
// made ready to be copied from (TRANSFER_SRC_OPTIMAL), and oxyos_paint_present's sequence returned
// (negative on failure, as oxyos_paint_present).
int64_t presentGpu(OxyOSPaint* paint, OxyOSClient* client, uint32_t surface, uint64_t configure,
                   const Canvas& canvas, uint32_t format, const std::vector<OxyOSRect>& damage);

}  // namespace oxyos
