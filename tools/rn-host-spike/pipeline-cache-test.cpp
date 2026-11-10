// Ganesh's persistent cache on disk (pipeline-cache.h), without a GPU: what is stored is read back
// by the next launch, whole and private; a damaged file is never given back and is removed; the
// bound removes the least recently used programs first and never the driver's pipeline cache;
// another GPU's or driver's directory is removed; and each program compiled is counted.
#include "pipeline-cache.h"

#include <include/core/SkString.h>

#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iostream>
#include <string>

namespace {

int failures = 0;

void check(bool ok, const char* what) {
  if (!ok) {
    std::cerr << "FAIL: " << what << "\n";
    ++failures;
  }
}

sk_sp<SkData> data(const std::string& text) { return SkData::MakeWithCopy(text.data(), text.size()); }

std::string text(const sk_sp<SkData>& data) {
  return data ? std::string(static_cast<const char*>(data->data()), data->size()) : std::string("(none)");
}

mode_t mode(const std::filesystem::path& path) {
  struct stat about{};
  ::stat(path.c_str(), &about);
  return about.st_mode & 0777;
}

uintmax_t total(const std::filesystem::path& directory) {
  uintmax_t bytes = 0;
  for (const auto& entry : std::filesystem::directory_iterator(directory)) bytes += entry.file_size();
  return bytes;
}

}  // namespace

int main() {
  const char* base = std::getenv("TMPDIR");
  std::string made = std::string(base ? base : "/tmp") + "/oxyos-pipeline-cache-XXXXXX";
  if (!::mkdtemp(made.data())) return 2;
  std::filesystem::path root = made;
  auto skia = root / "oxyos/apps/so.oxy.settings/skia";
  auto directory = skia / "8086-46a6-device";
  const uint32_t vkKey = 1;
  auto pipelineCacheKey = SkData::MakeWithCopy(&vkKey, sizeof vkKey);
  auto program = data("a program's description, as Ganesh keys it");

  {
    int compiled = 0;
    oxyos::PipelineCache cache(directory, [&compiled] { ++compiled; });
    cache.store(*program, *data("SPIR-V"), SkString());
    cache.store(*pipelineCacheKey, *data("the driver's pipeline cache"), SkString());
    check(compiled == 1, "a program stored is one compiled; the driver's cache is none");
    check(text(cache.load(*program)) == "SPIR-V", "what is stored is found before it is written");
    check(!std::filesystem::exists(directory), "nothing is written until write()");
    cache.write();
    check(mode(directory) == 0700, "the cache's directory is the account's alone");
    check(mode(root / "oxyos/apps/so.oxy.settings") == 0700, "so is each directory made for it");
    check(mode(cache.fileOf(*program)) == 0600, "a file is the account's alone");
  }
  {
    // The next launch.
    int compiled = 0;
    oxyos::PipelineCache cache(directory, [&compiled] { ++compiled; });
    check(text(cache.load(*program)) == "SPIR-V", "the next launch reads the program");
    check(text(cache.load(*pipelineCacheKey)) == "the driver's pipeline cache", "and the driver's cache");
    check(cache.load(*data("another program")) == nullptr, "a program never stored is not found");
    check(compiled == 0, "reading compiles nothing");
  }
  {
    // A file damaged, cut short, or of the format before.
    oxyos::PipelineCache cache(directory);
    auto file = cache.fileOf(*program);
    auto damage = [&](auto change) {
      oxyos::PipelineCache writer(directory);
      writer.store(*program, *data("SPIR-V"), SkString());
      writer.write();
      std::string bytes;
      {
        std::ifstream in(file, std::ios::binary);
        bytes.assign(std::istreambuf_iterator<char>(in), {});
      }
      change(bytes);
      std::ofstream(file, std::ios::binary | std::ios::trunc) << bytes;
      return cache.load(*program) == nullptr && !std::filesystem::exists(file);
    };
    check(damage([](std::string& bytes) { bytes.back() ^= 1; }), "a byte flipped: not read, removed");
    check(damage([](std::string& bytes) { bytes.resize(bytes.size() - 2); }), "cut short: not read, removed");
    check(damage([](std::string& bytes) { bytes += "x"; }), "longer: not read, removed");
    check(damage([](std::string& bytes) { bytes[7] = '1'; }), "the format before: not read, removed");
  }
  {
    // The bound: 16 programs of 1 KiB and the driver's cache within 6 KiB; the oldest go first, a
    // program read since is kept, the driver's cache always is.
    std::filesystem::remove_all(directory);
    oxyos::PipelineCache cache(directory, {}, 6 << 10);
    cache.store(*pipelineCacheKey, *data(std::string(1000, 'v')), SkString());
    cache.write();
    std::vector<sk_sp<SkData>> keys;
    for (int i = 0; i < 16; ++i) {
      keys.push_back(data("program " + std::to_string(i)));
      cache.store(*keys.back(), *data(std::string(1000, 'p')), SkString());
      cache.write();
      // Each older than the next, by a second.
      timespec times[2] = {{1000000 + i, 0}, {1000000 + i, 0}};
      ::utimensat(AT_FDCWD, cache.fileOf(*keys.back()).c_str(), times, 0);
    }
    check(total(directory) <= 6 << 10, "the directory stays within its bound");
    check(std::filesystem::exists(cache.fileOf(*pipelineCacheKey)), "the driver's cache is never removed");
    check(std::filesystem::exists(cache.fileOf(*keys[15])), "the newest program is kept");
    check(!std::filesystem::exists(cache.fileOf(*keys[0])), "the oldest is removed");
    // keys[12] read: the most recently used once written; then more programs come.
    oxyos::PipelineCache next(directory, {}, 6 << 10);
    check(next.load(*keys[12]) != nullptr, "a kept program is read");
    next.write();
    for (int i = 16; i < 19; ++i) {
      next.store(*data("program " + std::to_string(i)), *data(std::string(1000, 'p')), SkString());
      next.write();
    }
    check(std::filesystem::exists(next.fileOf(*keys[12])), "a program used is kept over older ones");
    check(!std::filesystem::exists(next.fileOf(*keys[15])), "one not used since goes");
  }
  {
    // Another GPU's, driver's or Skia's directory beside it.
    std::filesystem::create_directories(skia / "8086-46a6-old-driver");
    std::ofstream(skia / "8086-46a6-old-driver" / "0.bin") << "old";
    oxyos::PipelineCache cache(directory);
    cache.removeOthers();
    check(!std::filesystem::exists(skia / "8086-46a6-old-driver"), "another driver's cache is removed");
    check(std::filesystem::exists(directory), "this one is kept");
  }
  std::filesystem::remove_all(root);
  if (failures == 0) std::cout << "pipeline cache: every check passed\n";
  return failures == 0 ? 0 : 1;
}
