#!/usr/bin/env python3
"""Check that OxyOS's own C and C++ is formatted as .clang-format says.

Every C/C++ file tracked in this repository is OxyOS's own (the Bloom host in tools/rn-host-spike,
the client's C ABI header); third-party code is fetched at build time, never committed. Each must
be what clang-format 19.1.7 makes of it, except the files listed in scripts/cpp-format-baseline.txt,
which did not conform when the check began. The list only shrinks: a listed file that now conforms
(or is gone) fails the check until its line is removed. No file is reformatted here.

clang-format is CLANG_FORMAT, else `clang-format` on PATH; it must be 19.1.7 (other releases format
differently). One that is: `uvx --from clang-format==19.1.7 clang-format`, or
`pip install clang-format==19.1.7` in a virtual environment.

Usage: scripts/check-cpp-format.py [--write-baseline]
  --write-baseline  rewrite the baseline as the files that do not conform today (only to shrink it;
                    a new file is formatted, never listed)
"""

import concurrent.futures
import os
import pathlib
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASELINE = ROOT / "scripts" / "cpp-format-baseline.txt"
VERSION = "19.1.7"
SUFFIXES = ("*.c", "*.cc", "*.cpp", "*.h", "*.hpp")
HEADER = """\
# OxyOS's own C/C++ files that did not conform to .clang-format when scripts/check-cpp-format.py
# began. The list only shrinks: format a file (in a pull request of its own, its commit in
# .git-blame-ignore-revs) and remove its line. Never add one.
"""


def clang_format():
    binary = os.environ.get("CLANG_FORMAT") or shutil.which("clang-format")
    if not binary:
        sys.exit(f"clang-format {VERSION} not found: set CLANG_FORMAT (see this script's docstring)")
    version = subprocess.run([binary, "--version"], capture_output=True, text=True, check=True).stdout
    if f"version {VERSION}" not in version:
        sys.exit(f"{binary} is not clang-format {VERSION}: {version.strip()}")
    return binary


def own_sources():
    listed = subprocess.run(["git", "ls-files", "-z", "--", *SUFFIXES], cwd=ROOT, check=True,
                            capture_output=True, text=True).stdout
    return sorted(path for path in listed.split("\0") if path)


def read_baseline():
    if not BASELINE.exists():
        return []
    lines = BASELINE.read_text().splitlines()
    return [line.strip() for line in lines if line.strip() and not line.startswith("#")]


def conforms(binary, path):
    result = subprocess.run([binary, "--dry-run", "--Werror", "--style=file", path], cwd=ROOT,
                            capture_output=True, text=True)
    return result.returncode == 0


def main():
    binary = clang_format()
    sources = own_sources()
    with concurrent.futures.ThreadPoolExecutor() as pool:
        results = dict(zip(sources, pool.map(lambda path: conforms(binary, path), sources)))
    failing = [path for path, ok in results.items() if not ok]

    if sys.argv[1:] == ["--write-baseline"]:
        if set(failing) - set(read_baseline()) and BASELINE.exists():
            sys.exit("the baseline only shrinks: format the new files instead of listing them:\n  "
                     + "\n  ".join(sorted(set(failing) - set(read_baseline()))))
        BASELINE.write_text(HEADER + "".join(f"{path}\n" for path in failing))
        print(f"{BASELINE.relative_to(ROOT)}: {len(failing)} files")
        return 0
    if sys.argv[1:]:
        sys.exit(__doc__)

    baseline = read_baseline()
    problems = []
    for path in failing:
        if path not in baseline:
            problems.append(f"{path}: not formatted (run: clang-format -i {path})")
    for path in baseline:
        if path not in results:
            problems.append(f"{path}: listed in {BASELINE.name} but not a tracked C/C++ file; remove its line")
        elif results[path]:
            problems.append(f"{path}: now formatted; remove its line from {BASELINE.name}")
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    print(f"C/C++ format passed: {len(sources) - len(baseline)} files formatted, "
          f"{len(baseline)} in the baseline (clang-format {VERSION}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
