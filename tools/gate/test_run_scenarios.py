"""The scenario gate's own checks: `python3 tools/gate/test_run_scenarios.py`.

Gates on one machine share its CPU task by task (tools/gate/machine.sh), so one gate's build at
full priority took the CPU of another gate's running scenarios, which then failed late and passed
alone. tools/gate/run-scenarios.sh runs every build through `build` (at the least priority); these
check that it does, and that `build` and `step` run their command below the caller."""

import pathlib
import shlex
import re
import subprocess
import unittest

SCRIPT = pathlib.Path(__file__).with_name("run-scenarios.sh")
# The programs that build: the bootstrap, Rust's, C++'s and JavaScript's.
BUILDERS = {"bootstrap", "cargo", "cmake", "ninja", "npm", "node"}


def builds(line):
    """Whether the line runs a builder: a word of it (quoted or not) that names one."""
    try:
        words = shlex.split(line)
    except ValueError:
        words = line.split()
    return any(word.rsplit("/", 1)[-1] in BUILDERS for word in words)


def commands(text):
    """The script's lines that run something, comments and here-documents left out, continuations
    joined."""
    lines, heredoc, joined = [], None, ""
    for line in text.splitlines():
        if heredoc:
            if line.strip() == heredoc:
                heredoc = None
            continue
        start = re.search(r"<<-?\s*'?(\w+)'?", line)
        if start:
            heredoc = start.group(1)
        code = line.split(" #")[0] if not line.lstrip().startswith("#") else ""
        joined += code
        if joined.endswith("\\"):
            joined = joined[:-1]
            continue
        if joined.strip():
            lines.append(joined.strip())
        joined = ""
    return lines


def nice_of(shell_code):
    """The niceness a command run by `shell_code` (with run-scenarios.sh's `build` and `step`
    defined) has, and its caller's."""
    text = SCRIPT.read_text()
    definitions = "\n".join(re.findall(r"^(?:build|step|fail)\(\) \{.*\}$", text, re.M))
    out = subprocess.run(["sh", "-c", f"R=$(mktemp -d)\n{definitions}\n{shell_code}\ncat $R/*.log 2>/dev/null; rm -rf $R"],
                         capture_output=True, text=True, check=True).stdout.split()
    return [int(value) for value in out]


class Builds(unittest.TestCase):
    def test_every_build_runs_through_build(self):
        unbuilt = [line for line in commands(SCRIPT.read_text())
                   if builds(line) and not re.match(r"(step \S+|build) ", line)
                   and not line.startswith(". ")]
        self.assertEqual(unbuilt, [], "a build in run-scenarios.sh that is not through `build` or `step`")

    def test_build_and_step_run_below_the_caller(self):
        own = int(subprocess.run(["sh", "-c", "cut -d' ' -f19 /proc/$$/stat"], capture_output=True,
                                 text=True, check=True).stdout)
        wanted = min(own + 19, 19)
        (built,) = nice_of("build sh -c 'cut -d\" \" -f19 /proc/$$/stat'")
        (stepped,) = nice_of("step one.log sh -c 'cut -d\" \" -f19 /proc/$$/stat'")
        self.assertEqual(built, wanted)
        self.assertEqual(stepped, wanted)


if __name__ == "__main__":
    unittest.main()
