"""Lay out React Native's pinned third-party sources the way its Gradle tasks do.

Usage: prepare.py <downloaded-sources> <react-native-package> <output-dir>
"""
import pathlib, re, shutil, sys
tp, rn, out = (pathlib.Path(arg) for arg in sys.argv[1:4])
jni = rn / "ReactAndroid/src/main/jni/third-party"
shutil.rmtree(out, ignore_errors=True)
def layout(name, source, cmake=True):
    dest = out / name
    shutil.copytree(tp / source, dest / (source if name == "glog" else ""), dirs_exist_ok=True)
    if cmake:
        shutil.copy(jni / name / "CMakeLists.txt", dest / "CMakeLists.txt")
    return dest
# double-conversion: Gradle flattens src/** into double-conversion/.
dc = out / "double-conversion"
shutil.copytree(tp / "double-conversion-1.1.6/src", dc / "double-conversion")
shutil.copy(jni / "double-conversion/CMakeLists.txt", dc / "CMakeLists.txt")
for name, source in [("fast_float", "fast_float-8.0.0"),
                     ("fmt", "fmt-12.1.0"), ("folly", "folly-2024.11.18.00")]:
    layout(name, source)
# glog: tokens replaced in *.h.in, exported headers, config.h from RN.
tokens = {"ac_cv_have_unistd_h": "1", "ac_cv_have_stdint_h": "1", "ac_cv_have_systypes_h": "1",
          "ac_cv_have_inttypes_h": "1", "ac_cv_have_libgflags": "0", "ac_google_start_namespace": "namespace google {",
          "ac_cv_have_uint16_t": "1", "ac_cv_have_u_int16_t": "1", "ac_cv_have___uint16": "0",
          "ac_google_end_namespace": "}", "ac_cv_have___builtin_expect": "1", "ac_google_namespace": "google",
          "ac_cv___attribute___noinline": "__attribute__ ((noinline))", "ac_cv___attribute___noreturn": "__attribute__ ((noreturn))",
          "ac_cv___attribute___printf_4_5": "__attribute__((__format__ (__printf__, 4, 5)))"}
glog = out / "glog"
shutil.copytree(tp / "glog-0.3.5/src", glog / "glog-0.3.5/src")
# Gradle renames every *.h.in to its bare name at the glog root, so no src/config.h shadows
# React Native's config.h, which is copied last.
for template in list((glog / "glog-0.3.5/src").rglob("*.h.in")):
    text = re.sub(r"@(\w+)@", lambda m: tokens.get(m.group(1), m.group(0)), template.read_text())
    (glog / template.name[:-3]).write_text(text)
    template.unlink()
shutil.copy(jni / "glog/CMakeLists.txt", glog / "CMakeLists.txt")
shutil.copy(jni / "glog/config.h", glog / "config.h")
exported = glog / "exported/glog"; exported.mkdir(parents=True)
for header in ["stl_logging.h", "logging.h", "raw_logging.h", "vlog_is_on.h"]:
    shutil.copy(glog / header, exported / header)
shutil.copy(glog / "glog-0.3.5/src/glog/log_severity.h", exported / "log_severity.h")
# boost: headers only on Linux (the Android target only adds per-ABI assembly).
(out / "boost").mkdir()
(out / "boost/CMakeLists.txt").write_text(
    f"add_library(boost INTERFACE)\ntarget_include_directories(boost INTERFACE {tp / 'boost_1_83_0'})\n")
# Android builds folly against libc++; this host uses libstdc++.
folly_cmake = out / "folly/CMakeLists.txt"
folly_cmake.write_text(folly_cmake.read_text().replace("-DFOLLY_USE_LIBCPP=1", ""))
print("prepared", sorted(p.name for p in out.iterdir()))
