#include "memory.h"

#include "renderer.h"
#include "report.h"
#include "skia-gpu.h"

#include <jsi/instrumentation.h>
#include <jsi/jsi.h>
#include <oxyos_client.h>

#include <include/core/SkGraphics.h>
#include <include/core/SkTraceMemoryDump.h>
#include <include/gpu/ganesh/GrDirectContext.h>

#include <malloc.h>
#include <sys/prctl.h>

#include <cerrno>
#include <cmath>
#include <cstring>
#include <iostream>
#include <map>
#include <string>

namespace oxyos {

namespace {

// Allocations this large or larger are mapped by malloc on their own and unmapped when freed.
constexpr int kMmapThreshold = 4 << 20;

double mib(double bytes) { return std::round(bytes / (1 << 20) * 10) / 10; }

// Every GPU resource Skia holds, budgeted or not (a canvas is not), by its kind ("Texture",
// "RenderTarget", "Buffer", …), in bytes.
class ResourceSizes final : public SkTraceMemoryDump {
 public:
  void dumpNumericValue(const char* name, const char* value, const char*, uint64_t bytes) override {
    if (std::strcmp(value, "size") == 0) sizes_[name] += bytes;
  }
  void dumpStringValue(const char* name, const char* value, const char* kind) override {
    if (std::strcmp(value, "type") == 0) kinds_[name] = kind;
  }
  void setMemoryBacking(const char*, const char*, const char*) override {}
  void setDiscardableMemoryBacking(const char*, const SkDiscardableMemory&) override {}
  LevelOfDetail getRequestedDetails() const override { return kObjectsBreakdowns_LevelOfDetail; }

  std::map<std::string, uint64_t> byKind() const {
    std::map<std::string, uint64_t> out;
    for (const auto& [name, bytes] : sizes_) {
      auto kind = kinds_.find(name);
      out[kind == kinds_.end() ? "other" : kind->second] += bytes;
    }
    return out;
  }

 private:
  std::map<std::string, uint64_t> sizes_;
  std::map<std::string, std::string> kinds_;
};

}  // namespace

void configureProcessMemory() {
  // Inherited by whatever the host starts, as by the host from what started it.
  if (prctl(PR_SET_THP_DISABLE, 1, 0, 0, 0) != 0) {
    std::cerr << "[host] transparent huge pages stay on: " << std::strerror(errno) << "\n";
  }
  // Setting it fixes it: glibc no longer raises it to the size of each mapped buffer freed (up to
  // 32 MiB), nor the trim threshold with it.
  mallopt(M_MMAP_THRESHOLD, kMmapThreshold);
}

Memory& Memory::get() {
  static Memory memory;
  return memory;
}

void Memory::addRuntime(const char* name, std::function<facebook::jsi::Runtime*()> runtime) {
  runtimes_.emplace_back(name, std::move(runtime));
}

void Memory::worked(Clock::time_point now) {
  lastWork_ = now;
  trimmed_ = false;
}

std::optional<Memory::Clock::time_point> Memory::nextDeadline() const {
  if (trimmed_ || !lastWork_) return std::nullopt;
  return *lastWork_ + kSettledAfter;
}

bool Memory::tick(Clock::time_point now) {
  auto due = nextDeadline();
  if (!due || now < *due) return false;
  trimmed_ = true;
  if (trimBuffers_) trimBuffers_();
  if (GrDirectContext* context = skiaContext(gpuPaint())) {
    context->purgeUnlockedResources(GrPurgeResourceOptions::kScratchResourcesOnly);
    context->performDeferredCleanup(std::chrono::duration_cast<std::chrono::milliseconds>(kUnusedFor));
    storePipelineCache(context);
  }
  malloc_trim(0);
  if (Report::get().enabled()) Report::get().memory(account());
  return true;
}

folly::dynamic Memory::account() const {
  folly::dynamic out = folly::dynamic::object;
  // malloc's arenas (the main heap and each thread's) and its own mappings: what is in use, and
  // what it holds free (given back by malloc_trim, but for what fragments keep).
  struct mallinfo2 info = mallinfo2();
  out["malloc_in_use"] = mib(static_cast<double>(info.uordblks + info.hblkhd));
  out["malloc_free"] = mib(static_cast<double>(info.fordblks));
  for (const auto& [name, runtime] : runtimes_) {
    facebook::jsi::Runtime* rt = runtime();
    if (!rt) continue;
    auto heap = rt->instrumentation().getHeapInfo(false);
    auto value = [&](const char* key) { return static_cast<double>(heap.count(key) ? heap.at(key) : 0); };
    std::string prefix = std::string(name) + "_";
    // Its segments, what is allocated in them (live objects and garbage not yet collected), and
    // the memory its objects hold outside them (strings', arrays' and native objects' storage).
    out[prefix + "heap"] = mib(value("hermes_heapSize"));
    out[prefix + "allocated"] = mib(value("hermes_allocatedBytes"));
    out[prefix + "external"] = mib(value("hermes_externalBytes") + value("hermes_mallocSizeEstimate"));
  }
  if (GrDirectContext* context = skiaContext(gpuPaint())) {
    int resources = 0;
    size_t bytes = 0;
    context->getResourceCacheUsage(&resources, &bytes);
    out["skia_gpu"] = mib(static_cast<double>(bytes));
    out["skia_gpu_purgeable"] = mib(static_cast<double>(context->getResourceCachePurgeableBytes()));
    ResourceSizes sizes;
    context->dumpMemoryStatistics(&sizes);
    for (const auto& [kind, bytes] : sizes.byKind()) out["skia_all_" + kind] = mib(static_cast<double>(bytes));
  }
  out["skia_pictures"] = mib(static_cast<double>(SkGraphics::GetResourceCacheTotalBytesUsed()));
  out["skia_glyphs"] = mib(static_cast<double>(SkGraphics::GetFontCacheUsed()));
  uint64_t buffers = 0;
  uint32_t count = 0;
  if (OxyOSPaint* paint = gpuPaint(); paint && oxyos_paint_memory(paint, &buffers, &count) == 0) {
    out["buffers"] = mib(static_cast<double>(buffers));
    out["buffer_count"] = count;
  }
  return out;
}

}  // namespace oxyos
