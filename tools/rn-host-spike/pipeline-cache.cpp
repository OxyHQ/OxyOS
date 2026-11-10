#include "pipeline-cache.h"

#include <include/core/SkString.h>

#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>

#include <algorithm>
#include <cerrno>
#include <chrono>
#include <cstdio>
#include <cstring>

namespace oxyos {

namespace {

constexpr char kMagic[8] = {'O', 'X', 'S', 'K', 'C', 'A', 'C', '2'};
// The magic, the key's length and the data's (4 bytes each), the checksum (8).
constexpr size_t kHeader = sizeof kMagic + 4 + 4 + 8;
// The key Ganesh keeps the driver's pipeline cache under: GrVkGpu::kPipelineCache_PersistentCacheKeyType
// alone (a program's key is its description, longer).
constexpr uint32_t kVkPipelineCacheKey = 1;

uint64_t fnv1a(const void* bytes, size_t size, uint64_t hash = 0xcbf29ce484222325ull) {
  for (size_t i = 0; i < size; ++i) hash = (hash ^ static_cast<const uint8_t*>(bytes)[i]) * 0x100000001b3ull;
  return hash;
}

// Makes `directory` and its parents, each this account's alone (0700) where it is made, and the
// last 0700 whatever it was.
bool makePrivateDirectory(const std::filesystem::path& directory) {
  std::error_code error;
  if (!std::filesystem::is_directory(directory, error)) {
    if (directory.has_parent_path() && directory.parent_path() != directory &&
        !makePrivateDirectory(directory.parent_path())) {
      return false;
    }
    if (::mkdir(directory.c_str(), 0700) != 0 && errno != EEXIST) return false;
  }
  return ::chmod(directory.c_str(), 0700) == 0;
}

}  // namespace

PipelineCache::PipelineCache(std::filesystem::path directory, std::function<void()> compiled, size_t maxBytes)
    : directory_(std::move(directory)), compiled_(std::move(compiled)), maxBytes_(maxBytes) {}

bool PipelineCache::isVkPipelineCacheKey(const SkData& key) {
  return key.size() == sizeof kVkPipelineCacheKey &&
         std::memcmp(key.data(), &kVkPipelineCacheKey, sizeof kVkPipelineCacheKey) == 0;
}

std::filesystem::path PipelineCache::fileOf(const SkData& key) const {
  char name[32];
  std::snprintf(name, sizeof name, "%016llx.bin", static_cast<unsigned long long>(fnv1a(key.data(), key.size())));
  return directory_ / name;
}

sk_sp<SkData> PipelineCache::load(const SkData& key) {
  std::string wanted(static_cast<const char*>(key.data()), key.size());
  if (auto it = pending_.find(wanted); it != pending_.end()) return it->second;
  auto path = fileOf(key);
  int fd = ::open(path.c_str(), O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
  if (fd < 0) return nullptr;
  struct stat about{};
  std::string bytes;
  if (::fstat(fd, &about) == 0 && S_ISREG(about.st_mode) && static_cast<size_t>(about.st_size) <= maxBytes_) {
    bytes.resize(static_cast<size_t>(about.st_size));
    size_t done = 0;
    while (done < bytes.size()) {
      ssize_t n = ::read(fd, bytes.data() + done, bytes.size() - done);
      if (n <= 0) break;
      done += static_cast<size_t>(n);
    }
    bytes.resize(done);
  }
  ::close(fd);
  uint32_t keyLength = 0, dataLength = 0;
  uint64_t sum = 0;
  bool whole = bytes.size() >= kHeader && std::memcmp(bytes.data(), kMagic, sizeof kMagic) == 0;
  if (whole) {
    std::memcpy(&keyLength, bytes.data() + 8, 4);
    std::memcpy(&dataLength, bytes.data() + 12, 4);
    std::memcpy(&sum, bytes.data() + 16, 8);
    whole = bytes.size() == kHeader + static_cast<size_t>(keyLength) + dataLength &&
            fnv1a(bytes.data() + kHeader, bytes.size() - kHeader) == sum;
  }
  if (!whole) {
    // Damaged, or of another format: made again.
    ::unlink(path.c_str());
    return nullptr;
  }
  // Another key with the same hash: its file is replaced once this key's data is stored.
  if (bytes.compare(kHeader, keyLength, wanted) != 0) return nullptr;
  used_.push_back(path);
  return SkData::MakeWithCopy(bytes.data() + kHeader + keyLength, dataLength);
}

void PipelineCache::store(const SkData& key, const SkData& data, const SkString&) {
  // A program's shaders are stored once, when Ganesh compiled them.
  if (!isVkPipelineCacheKey(key) && compiled_) compiled_();
  pending_[std::string(static_cast<const char*>(key.data()), key.size())] =
      SkData::MakeWithCopy(data.data(), data.size());
}

void PipelineCache::write() {
  if (pending_.empty() && used_.empty()) return;
  if (!makePrivateDirectory(directory_)) {
    pending_.clear();
    used_.clear();
    return;
  }
  for (const auto& [key, data] : pending_) writeFile(fileOf(*SkData::MakeWithoutCopy(key.data(), key.size())), key, *data);
  pending_.clear();
  // What was read since is the most recently used (its file's time), so the bound removes the rest
  // first.
  for (const auto& path : used_) ::utimensat(AT_FDCWD, path.c_str(), nullptr, 0);
  used_.clear();
  trim();
}

void PipelineCache::removeOthers() const {
  std::error_code error;
  for (const auto& entry : std::filesystem::directory_iterator(directory_.parent_path(), error)) {
    if (entry.path() != directory_) std::filesystem::remove_all(entry.path(), error);
  }
}

void PipelineCache::writeFile(const std::filesystem::path& path, const std::string& key, const SkData& data) const {
  if (kHeader + key.size() + data.size() > maxBytes_) return;
  std::string aside = (directory_ / ".new-XXXXXX").string();
  int fd = ::mkstemp(aside.data());  // 0600
  if (fd < 0) return;
  uint32_t keyLength = static_cast<uint32_t>(key.size()), dataLength = static_cast<uint32_t>(data.size());
  uint64_t sum = fnv1a(data.data(), data.size(), fnv1a(key.data(), key.size()));
  char header[kHeader];
  std::memcpy(header, kMagic, sizeof kMagic);
  std::memcpy(header + 8, &keyLength, 4);
  std::memcpy(header + 12, &dataLength, 4);
  std::memcpy(header + 16, &sum, 8);
  auto all = [fd](const void* bytes, size_t size) {
    for (size_t done = 0; done < size;) {
      ssize_t n = ::write(fd, static_cast<const char*>(bytes) + done, size - done);
      if (n <= 0) return false;
      done += static_cast<size_t>(n);
    }
    return true;
  };
  bool written = all(header, sizeof header) && all(key.data(), key.size()) && all(data.data(), data.size());
  written = ::close(fd) == 0 && written;
  if (!written || ::rename(aside.c_str(), path.c_str()) != 0) ::unlink(aside.c_str());
}

void PipelineCache::trim() const {
  struct Entry {
    std::filesystem::path path;
    std::filesystem::file_time_type time;
    uintmax_t size;
  };
  std::vector<Entry> programs;
  uintmax_t total = 0;
  std::error_code error;
  auto now = std::filesystem::file_time_type::clock::now();
  auto pipelineCache = fileOf(*SkData::MakeWithoutCopy(&kVkPipelineCacheKey, sizeof kVkPipelineCacheKey));
  for (const auto& entry : std::filesystem::directory_iterator(directory_, error)) {
    std::error_code about;
    auto time = entry.last_write_time(about);
    if (about) continue;
    // Left aside by a process that ended between writing and renaming: removed once a minute old.
    if (entry.path().filename().string().rfind(".new-", 0) == 0) {
      if (now - time > std::chrono::minutes(1)) std::filesystem::remove(entry.path(), about);
      continue;
    }
    auto size = entry.file_size(about);
    if (about) continue;
    total += size;
    if (entry.path() != pipelineCache) programs.push_back({entry.path(), time, size});
  }
  if (total <= maxBytes_) return;
  std::sort(programs.begin(), programs.end(), [](const Entry& a, const Entry& b) { return a.time < b.time; });
  for (const auto& entry : programs) {
    if (total <= maxBytes_) break;
    std::error_code removed;
    if (std::filesystem::remove(entry.path, removed)) total -= entry.size;
  }
}

}  // namespace oxyos
