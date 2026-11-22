#!/usr/bin/env python3
"""Validate libtest's captured pretty output against an exact discovered set."""
import argparse
from collections import Counter
from pathlib import Path
import re
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("kind", choices=("list", "run"))
parser.add_argument("log", type=Path)
parser.add_argument("exit_code", type=int)
parser.add_argument("tests", nargs="+")
args = parser.parse_args()
text = args.log.read_text(encoding="utf-8", errors="replace")
errors = []
if args.exit_code:
    errors.append(f"command exited with status {args.exit_code}")
if re.search(r"\bSKIP\b", text):
    errors.append("tests reported SKIP")

if args.kind == "list":
    names = re.findall(r"^([^\s]+): test$", text, re.MULTILINE)
    summaries = re.findall(r"^(\d+) tests?, (\d+) benchmarks?$", text, re.MULTILINE)
    if summaries != [(str(len(names)), "0")]:
        errors.append("test listing has a missing or inconsistent summary")
else:
    # --show-output keeps test diagnostics out of the result records.
    records = re.findall(r"^test ([^\s]+) \.\.\. (ok|FAILED|ignored(?:, .*)?)$",
                         text, re.MULTILINE)
    names = [name for name, _ in records]
    passed = sum(status == "ok" for _, status in records)
    failed = sum(status == "FAILED" for _, status in records)
    ignored = sum(status.startswith("ignored") for _, status in records)
    executed = passed + failed
    starts = re.findall(r"^running (\d+) tests?$", text, re.MULTILINE)
    summaries = re.findall(
        r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; "
        r"(\d+) measured; (\d+) filtered out(?:; finished in .*)?$", text, re.MULTILINE)
    if starts != [str(len(args.tests))]:
        errors.append("test run has a missing or inconsistent running count")
    if len(summaries) != 1:
        errors.append("test run must have exactly one result summary")
    else:
        result, summary_passed, summary_failed, summary_ignored, measured, _ = summaries[0]
        if (int(summary_passed), int(summary_failed), int(summary_ignored), int(measured)) != (
                passed, failed, ignored, 0):
            errors.append("result summary disagrees with individual test records")
        if result != "ok":
            errors.append("test result is FAILED")
    if passed != len(args.tests) or failed or ignored:
        errors.append(f"expected {len(args.tests)} passes; got {passed} passed, "
                      f"{failed} failed, {ignored} ignored")
    print(executed, passed, ignored)

actual = Counter(names)
expected = Counter(args.tests)
for name in sorted(expected.keys() - actual.keys()):
    errors.append(f"missing test: {name}")
for name in sorted(actual.keys() - expected.keys()):
    errors.append(f"unexpected test: {name}")
for name, count in actual.items():
    if count != 1:
        errors.append(f"duplicate test ({count} records): {name}")
for error in errors:
    print(f"{args.log.name}: {error}", file=sys.stderr)
sys.exit(bool(errors))
