#!/usr/bin/env python3
"""Vendors KTX-Software-CTS test cases of one ktx command into tests/resources/cts.

Copies tests/<command>/*.json and every file referenced by a subcase (inputs
named in the command line, stdin, golden stdout/stderr and reference outputs).
Usage: scripts/vendor_cts.py <command>
"""
import itertools
import json
import os
import re
import shutil
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "ref", "ktx", "tests", "cts", "clitests")
DST = os.path.join(ROOT, "tests", "resources", "cts")


def evaluate(text, args):
    def repl(m):
        v = args[m.group(1)]
        if m.group(2):
            v = v[m.group(2).strip("'")]
        if isinstance(v, bool):
            return "True" if v else "False"
        return str(v)
    return re.sub(r"\$\{([a-zA-Z0-9_\-]+)(?:\[([a-zA-Z0-9_\-']+)\])?\}", repl, text)


def main():
    command = sys.argv[1]
    files = set()
    tests_dir = os.path.join(SRC, "tests", command)
    for name in sorted(os.listdir(tests_dir)):
        if not name.endswith(".json"):
            continue
        files.add(os.path.join("tests", command, name))
        with open(os.path.join(tests_dir, name)) as f:
            case = json.load(f)
        names = list(case.get("args", {}).keys())
        for combo in itertools.product(*(case["args"][n] for n in names)):
            args = dict(zip(names, combo))
            cmd = evaluate(case["command"], args)
            stdin = evaluate(case["stdin"], args) if "stdin" in case else None
            refs = [t for t in cmd.split(" ") if os.path.isfile(os.path.join(SRC, t))]
            if stdin:
                refs.append(stdin)
            for key in ("stdout", "stderr"):
                if key in case:
                    refs.append(evaluate(case[key], args))
            for ref in case.get("outputs", {}).values():
                refs.append(evaluate(ref, args))
            files.update(r for r in refs if os.path.isfile(os.path.join(SRC, r)))
    copied = 0
    for rel in sorted(files):
        dst = os.path.join(DST, rel)
        if not os.path.exists(dst):
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            shutil.copyfile(os.path.join(SRC, rel), dst)
            copied += 1
    print(f"{command}: {len(files)} files referenced, {copied} copied")


if __name__ == "__main__":
    main()
