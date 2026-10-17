"""Offline #39 matched recorder, using the unchanged #38 corpus and timing seams.

From repository root:
  python docs/benchmarks/integer-add-sub-39/record_matched.py --self-test
  python docs/benchmarks/integer-add-sub-39/record_matched.py docs/benchmarks/integer-add-sub-39/NEW-RUN

New output directories only. Commands run sequentially with 120-second deadlines;
Node retains its 60-second bound. Neither bound is guest fuel/process-tree control.
Guest samples and complete Divan reports are preserved. Divan 0.1.21's public CLI
reports aggregates, not individual compiler samples; no raw compiler samples are
invented. Compiler summary values retain the CLI's rounded precision.
"""

import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import unittest

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("baseline", ROOT / "tests/support/record_integer_baseline.py")
baseline = importlib.util.module_from_spec(spec)
spec.loader.exec_module(baseline)
LEVELS = ("None", "Basic")
NOTE = ("Matched experimental #39 observations, not release validation. Locals remain f64; "
        "eligible exact-i32 literals/add-sub trees may specialize. Power, thermals, "
        "background activity and settled V8 tiering are not controlled. Guest timing "
        "includes JS/Wasm calls and effects bookkeeping. Compiler reports contain "
        "Divan aggregates (100 samples, one compilation per sample), not raw samples.")


def hashes():
    result = baseline.hashes(ROOT)
    result["measurement"][Path(__file__).relative_to(ROOT).as_posix()] = baseline.hashlib.sha256(
        Path(__file__).read_bytes()).hexdigest()
    return result


def changed(before, after):
    return [f"{group}/{name}" for group in sorted(before.keys() | after.keys())
            for name in sorted(before.get(group, {}).keys() | after.get(group, {}).keys())
            if before.get(group, {}).get(name) != after.get(group, {}).get(name)]


def compiler_records(text):
    """Parse the documented Divan table; reject partial or duplicate results."""
    rows, stage, sizes = {}, None, {}
    units = {"ns": .001, "µs": 1, "μs": 1, "ms": 1000, "s": 1000000}
    for line in text.splitlines():
        size = re.fullmatch(r"([a-z0-9_]+),(\d+),(\d+)", line)
        if size:
            name, source, module = size.groups()
            baseline.require(name not in sizes, "Duplicate compiler size")
            sizes[name] = [int(source), int(module)]
        if re.search(r"(?:source_compile|typed_compile)\s+│", line):
            stage = "source_compile" if "source_compile" in line else "typed_compile"
        match = re.search(r"(?:├─|╰─)\s+(\w+)\s+([\d.]+)\s+(\S+)\s*│", line)
        if not match or stage is None:
            continue
        columns = line[match.start():].split("│")
        baseline.require(len(columns) == 6, "Unexpected Divan columns")
        name, fastest, unit = match.groups()
        baseline.require(unit in units, "Unknown compiler timing unit")
        values = [float(fastest) * units[unit]]
        for column in columns[1:4]:
            value, unit = column.split()
            baseline.require(unit in units, "Unknown compiler timing unit")
            values.append(float(value) * units[unit])
        baseline.require([int(c.strip()) for c in columns[4:]] == [100, 100],
                         "Expected 100 samples and 100 iterations")
        key = (stage, name)
        baseline.require(key not in rows, "Duplicate compiler row")
        rows[key] = dict(zip(("min_us", "max_us", "median_us", "mean_us"), values))
    baseline.require(set(rows) == {(s, n) for s in ("source_compile", "typed_compile")
                                  for n in baseline.NAMES}, "Incomplete compiler report")
    baseline.require(set(sizes) == set(baseline.NAMES), "Incomplete compiler sizes")
    return rows, sizes


def identities(verify, guests):
    """Sources match across levels; modules match within a level, not across None/Basic."""
    for name in baseline.NAMES:
        reference = verify[("None", name)]
        for level in baseline.LEVELS:
            row = verify[(level, name)]
            baseline.require(all(row[k] == reference[k] for k in ("sourceBytes", "sourceSha256")),
                             f"Cross-level source mismatch: {level}/{name}")
        for level in ("Standard", "Aggressive"):
            baseline.require(all(verify[(level, name)][k] == verify[("Basic", name)][k]
                                 for k in baseline.IDENTITY), f"Enabled preset mismatch: {name}")
        for level, records in guests:
            baseline.require(all(records[(level, name)][k] == verify[(level, name)][k]
                                 for k in baseline.IDENTITY), f"Within-level mismatch: {level}/{name}")


def summarize(out):
    verify, machine = baseline.parse_records((out / "verify.txt").read_text(encoding="utf-8"),
                                             "verify", baseline.LEVELS)
    guests, compilers = {}, {}
    for i in (1, 2):
        for level in LEVELS:
            stem = f"{i}-{level}"
            guests[(i, level)] = baseline.parse_records(
                (out / f"guest-{stem}.txt").read_text(encoding="utf-8"), "timing", (level,))
            compilers[(i, level)] = compiler_records(
                (out / f"compiler-{stem}.txt").read_text(encoding="utf-8"))
    identities(verify, [(level, records) for (_, level), (records, _) in guests.items()])
    machines = [machine] + [m for _, m in guests.values()]
    fields = ("node", "v8", "platform", "arch", "osRelease", "cpu", "logicalCpus",
              "totalMemoryBytes", "nodeFlags")
    baseline.require(all(all(m[k] == machine[k] for k in fields) for m in machines),
                     "Machine/runtime changed")
    historical = json.loads((ROOT / "docs/benchmarks/integer-specialization/summary.json").read_text())
    historical = {row["name"]: row for row in historical["workloads"]}
    workloads = []
    for name in baseline.NAMES:
        row = {"name": name, "levels": {}}
        baseline.require(all(verify[("None", name)][k] == historical[name][k] for k in baseline.IDENTITY),
                         f"None differs from historical #38 source/module identity: {name}")
        for level in LEVELS:
            reference = verify[(level, name)]
            for i in (1, 2):
                baseline.require(compilers[(i, level)][1][name] ==
                                 [reference["sourceBytes"], reference["moduleBytes"]],
                                 f"Compiler/guest size mismatch: {level}/{name}")
            row["levels"][level] = {
                **{k: reference[k] for k in baseline.IDENTITY},
                "guest_medians_ns": [guests[(i, level)][0][(level, name)].get("guestTiming", {}).get("median")
                                     for i in (1, 2)],
                "compiler": {stage: [compilers[(i, level)][0][(stage, name)] for i in (1, 2)]
                             for stage in ("source_compile", "typed_compile")},
            }
        row["module_delta_bytes"] = row["levels"]["Basic"]["moduleBytes"] - row["levels"]["None"]["moduleBytes"]
        workloads.append(row)
    baseline.write_json(out / "summary.json", {"notes": NOTE, "verify_records": 108,
        "enabled_presets_identical": True, "None_matches_historical_38": True,
        "machines": machines, "workloads": workloads})
    lines = ["# Matched integer literal/add-sub measurements (#39)", "", NOTE, "",
             "108 independent-oracle checks passed. Enabled presets match each other; None",
             "matches all 27 historical #38 source/module hashes. Matched measurements use",
             "the current tree, not historical timings. Each level has two compiler runs",
             "(54 cases/run) and two guest runs (26 timed + one verify-only trap/run).", "",
             "All pairs below are run 1 / run 2. Compiler medians are µs/compilation;",
             "guest medians are ns/main call. See raw logs for spread and guest samples.", "",
             "| Workload | Bytes None → Basic (delta) | Source None | Source Basic | Typed None | Typed Basic | Guest None | Guest Basic |",
             "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for row in workloads:
        n, b = (row["levels"][level] for level in LEVELS)
        def pair(values):
            return " / ".join("verify-only" if v is None else f"{v:g}" for v in values)
        cells = [f"{n['moduleBytes']} → {b['moduleBytes']} ({row['module_delta_bytes']:+})"]
        for stage in ("source_compile", "typed_compile"):
            cells += [pair([v["median_us"] for v in data["compiler"][stage]]) for data in (n, b)]
        cells += [pair(data["guest_medians_ns"]) for data in (n, b)]
        lines.append("| " + row["name"] + " | " + " | ".join(cells) + " |")
    (out / "summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main(argument):
    base = Path(__file__).resolve().parent
    candidate = (ROOT / argument).resolve()
    baseline.require(candidate.is_relative_to(base) and candidate != base,
                     "Output must be a NEW child of docs/benchmarks/integer-add-sub-39")
    out = baseline.create_output(ROOT, argument)
    env = dict(os.environ, CARGO_NET_OFFLINE="true")
    baseline.write_json(out / "environment.json", {"utc": baseline.utc(), "command": sys.argv,
        "cwd": str(ROOT), "python": sys.version, "notes": NOTE,
        "environment": {k: v for k, v in env.items() if k.startswith(("CARGO", "RUST", "DIVAN", "INTEGER"))
                        or k in ("NODE_OPTIONS", "PATH", "PROCESSOR_IDENTIFIER", "NUMBER_OF_PROCESSORS")},
        "order": "verify; compiler None/Basic + guest None/Basic; compiler Basic/None + guest Basic/None",
        "overrides": {"CARGO_NET_OFFLINE": "true", "INTEGER_BASELINE_LEVEL": "set per command"}})
    before = hashes()
    baseline.write_json(out / "hashes-before.json", before)
    completed = False
    def run(name, command, level="None"):
        start = hashes()
        baseline.require(start == before, f"Inputs changed before {name}; need stable production")
        baseline.write_json(out / f"inputs-{name}-before.json", start)
        try:
            baseline.record(out, name, command, dict(env, INTEGER_BASELINE_LEVEL=level))
        finally:
            end = hashes()
            baseline.write_json(out / f"inputs-{name}-after.json", end)
            baseline.require(end == before, f"Inputs changed during {name}; need stable production")
    try:
        for name, command in (
            ("git-head", ["git", "--no-pager", "rev-parse", "HEAD"]),
            ("git-status-before", ["git", "--no-pager", "--no-optional-locks", "status", "--short", "--untracked-files=all"]),
            ("git-diff-before", ["git", "--no-pager", "diff", "--binary"]),
            ("rustc", ["rustc", "-Vv"]), ("cargo", ["cargo", "--version"]),
            ("node", ["node", "--version"]),
        ):
            run(name, command)
        test = ["cargo", "test", "--offline", "-p", "was-compiler", "--test", "integer_baselines"]
        tail = ["--nocapture", "--test-threads=1"]
        run("default-test", test + ["--"] + tail)
        run("verify", test + ["execute_integer_workloads", "--", "--ignored", "--exact"] + tail)
        baseline.parse_records((out / "verify.txt").read_text(encoding="utf-8"), "verify", baseline.LEVELS)
        for i, order in ((1, LEVELS), (2, tuple(reversed(LEVELS)))):
            for level in order:
                run(f"compiler-{i}-{level}", ["cargo", "bench", "--offline", "-p", "was-compiler",
                    "--bench", "integer_specialization", "--", "--sample-count", "100",
                    "--sample-size", "1", "--timer", "os", "--color", "never"], level)
            for level in order:
                run(f"guest-{i}-{level}", test + ["record_integer_guest_timing", "--", "--ignored",
                    "--exact"] + tail, level)
        run("git-status-after", ["git", "--no-pager", "--no-optional-locks", "status", "--short", "--untracked-files=all"])
        completed = True
    finally:
        after = hashes()
        baseline.write_json(out / "hashes-after.json", after)
        baseline.write_json(out / "stability.json", {"utc_finished": baseline.utc(),
            "commands_completed": completed, "inputs_unchanged": before == after,
            "changed_inputs": changed(before, after)})
    baseline.require(completed and before == after, "Incomplete/unstable evidence; no summary")
    summarize(out)
    print("Matched collection complete:", out)


def self_test():
    class Tests(unittest.TestCase):
        def test_compiler_report_and_rejections(self):
            fixture = (ROOT / "docs/benchmarks/integer-specialization/compiler-1.txt").read_text(encoding="utf-8")
            rows, sizes = compiler_records(fixture)
            self.assertAlmostEqual(rows[("source_compile", "literal_32")]["median_us"], 146.3)
            self.assertEqual(sizes["literal_32"], [861, 1156])
            for bad in (fixture + fixture, fixture.replace("│ 100     │ 100", "│ 99      │ 100"),
                        fixture.replace("literal_32", "not_a_case")):
                with self.assertRaises(ValueError):
                    compiler_records(bad)

        def test_preset_identity_and_within_level_stability(self):
            verify = {(level, name): {"sourceBytes": 1, "sourceSha256": "a" * 64,
                       "moduleBytes": 10 if level == "None" else 5,
                       "moduleSha256": ("b" if level == "None" else "c") * 64}
                      for level in baseline.LEVELS for name in baseline.NAMES}
            guest = {("Basic", name): dict(verify[("Basic", name)]) for name in baseline.NAMES}
            identities(verify, [("Basic", guest)])
            guest[("Basic", "literal_1")]["moduleBytes"] = 6
            with self.assertRaises(ValueError):
                identities(verify, [("Basic", guest)])
            verify[("Standard", "literal_1")]["moduleBytes"] = 6
            with self.assertRaises(ValueError):
                identities(verify, [])

        def test_changed_inputs_including_added_removed(self):
            self.assertEqual(changed({"p": {"a": "1", "b": "2"}}, {"p": {"a": "3", "c": "4"}}),
                             ["p/a", "p/b", "p/c"])
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(self_test())
    baseline.require(len(sys.argv) == 2, __doc__)
    main(sys.argv[1])
