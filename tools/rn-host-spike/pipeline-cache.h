// Ganesh's persistent cache on disk (GrContextOptions::PersistentCache), as Chrome's and Flutter's
// (skia-gpu.h, docs/adr/0031-gpu-buffers.mdx, decision 12): each program's shaders (SPIR-V) Ganesh
// compiled and the driver's Vulkan pipeline cache, kept in one directory of the application's
// cache directory for its GPU, driver and Skia's revision (skia-gpu.cpp names it), so that an
// application's later launches compile no pipeline.
//
// - **One file a key**, named by the key's hash: a magic, the key's length and the data's, a
//   checksum of both (FNV-1a), the key, then the data. The key is checked whole, so two keys never
//   share data; a file whose magic, lengths or checksum are wrong (cut short, damaged, of another
//   format) is removed, never given to Ganesh or the driver.
// - **Never written while a frame is drawn**: what Ganesh stores is held in memory until `write`
//   (once the window is idle, and as its loop ends), and each file is written aside and renamed,
//   so the application's other processes read the file before or this one, whole.
// - **Private**: the directories 0700 where they are made (the last whatever it was), the files
//   0600, in the account's own cache directory: never read by another account.
// - **Bounded** at kMaxBytes: the driver's pipeline cache within half (Ganesh is asked for no
//   more), the programs' files within the rest, the least recently used (read or written) removed
//   first.
#pragma once

#include <include/core/SkData.h>
#include <include/gpu/ganesh/GrContextOptions.h>

#include <filesystem>
#include <functional>
#include <map>
#include <string>
#include <vector>

namespace oxyos {

class PipelineCache final : public GrContextOptions::PersistentCache {
 public:
  // An application's at most (Settings' after its first launch on the Iris Xe: about 100 KiB).
  static constexpr size_t kMaxBytes = 32u << 20;

  // The cache in `directory`; `compiled` is called for each program whose shaders Ganesh compiled
  // (stored), for the frame's diagnostics.
  explicit PipelineCache(std::filesystem::path directory, std::function<void()> compiled = {},
                         size_t maxBytes = kMaxBytes);

  sk_sp<SkData> load(const SkData& key) override;
  void store(const SkData& key, const SkData& data, const SkString& description) override;

  // Writes what Ganesh stored since, marks what it read as used, and keeps the directory within
  // its bound.
  void write();
  // Removes every other directory beside this one: another GPU's, driver's or Skia revision's,
  // never read again.
  void removeOthers() const;

  // The file `key` is kept in.
  std::filesystem::path fileOf(const SkData& key) const;
  // Whether `key` is the one Ganesh keeps the driver's pipeline cache under.
  static bool isVkPipelineCacheKey(const SkData& key);

 private:
  void writeFile(const std::filesystem::path& path, const std::string& key, const SkData& data) const;
  void trim() const;

  std::filesystem::path directory_;
  std::function<void()> compiled_;
  size_t maxBytes_;
  std::map<std::string, sk_sp<SkData>> pending_;
  std::vector<std::filesystem::path> used_;
};

}  // namespace oxyos
