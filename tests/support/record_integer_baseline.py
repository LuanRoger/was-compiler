"""Offline #38 baseline recorder; run from repository root with a NEW directory.

Usage: python tests/support/record_integer_baseline.py docs/benchmarks/NEW-EMPTY-DIR
       python tests/support/record_integer_baseline.py --self-test

Only the self-tests use temporary directories. Measurement commands are sequential,
with a 120-second deadline each. Deadlines kill/reap the direct child, not its
process tree, and are not guest fuel. Existing evidence is never overwritten.
"""

import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
LEVELS = ("None", "Basic", "Standard", "Aggressive")
NAMES = tuple(f"{family}_{size}" for size in (1, 32)
              for family in ("literal", "locals", "compare", "negate")) + (
    "loop_local", "fraction", "negative_zero", "negate_zero", "positive_zero",
    "overflow", "underflow_i32", "negate_min", "unknown", "mutable", "capture",
    "multiply", "divide", "remainder", "nan", "infinity", "subnormal", "effects",
    "trap_order",
)
IDENTITY = ("sourceBytes", "moduleBytes", "sourceSha256", "moduleSha256")
NOTE = ("Current presets are identical no-ops. These baseline observations make no "
        "enabled optimization benefit or release-validation claim. Power, thermal "
        "state and background activity are uncontrolled. Compiler timings and guest "
        "timings are separate; complete compiler output is retained without parsing.")


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def write_json(path, value):
    with path.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write("\n")


def create_output(root, argument):
    require(Path.cwd().resolve() == root.resolve(), "Run from repository root")
    base = (root / "docs/benchmarks").resolve()
    out = (root / argument).resolve()
    require(out != base and out.is_relative_to(base),
            "Evidence must be a new directory under docs/benchmarks")
    # No exist_ok: even an existing empty directory is not new evidence storage.
    out.mkdir(parents=True, exist_ok=False)
    return out


def hashes(root):
    production = list(root.glob("crates/*/src/**/*.rs"))
    production += [p for p in (root / "runtime").rglob("*") if p.is_file()]
    production += [root / "Cargo.toml", root / "Cargo.lock"]
    production += list(root.glob("crates/*/Cargo.toml"))
    measured = [root / p for p in (
        "tests/support/integer_corpus.rs", "tests/support/integer_measure.cjs",
        "tests/support/process.rs", "tests/support/record_integer_baseline.py",
        "crates/compiler/tests/integer_baselines.rs",
        "crates/compiler/benches/integer_specialization.rs",
    )]
    return {group: {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
                    for p in sorted(set(paths))}
            for group, paths in (("production", production), ("measurement", measured))}


def record(out, name, command, env):
    """Keep raw combined bytes intact, including partial output on failure/timeout."""
    metadata = {"command": command, "utc_started": utc(), "timeout_seconds": 120,
                "exit_code": None, "timed_out": False, "output": name + ".txt"}
    print("COMMAND:", " ".join(command), flush=True)
    error = None
    with (out / (name + ".txt")).open("xb") as stream:
        try:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=stream,
                                    stderr=subprocess.STDOUT, timeout=120, check=False)
            metadata["exit_code"] = result.returncode
        except subprocess.TimeoutExpired as exc:
            metadata["timed_out"] = True
            error = str(exc)
        except OSError as exc:
            error = str(exc)
    metadata["utc_finished"] = utc()
    if error:
        metadata["error"] = error
    write_json(out / (name + ".json"), metadata)
    if error or metadata["exit_code"] != 0:
        raise RuntimeError(f"{name} failed; see {name}.json and preserved raw output")
    return metadata


def parse_records(text, mode, levels):
    """Accept libtest's first-line prefix, but never silently collapse duplicates."""
    rows, machines = {}, []
    for line in text.splitlines():
        start = line.find("{")
        if start < 0:
            continue
        try:
            row = json.loads(line[start:])
        except json.JSONDecodeError:
            continue
        if not isinstance(row, dict):
            continue
        if "mode" in row:
            require(row["mode"] == mode, "Unexpected guest mode")
            machines.append(row)
        if "name" not in row:
            continue
        key = (row.get("level"), row["name"])
        require(key not in rows, f"Duplicate guest record: {key}")
        for field in ("sourceBytes", "moduleBytes"):
            require(type(row.get(field)) is int and row[field] > 0, f"Invalid {field}: {key}")
        for field in ("sourceSha256", "moduleSha256"):
            require(isinstance(row.get(field), str) and
                    re.fullmatch(r"[0-9a-f]{64}", row[field]), f"Invalid {field}: {key}")
        timing = row.get("guestTiming")
        if mode == "timing" and row["name"] != "trap_order":
            require(isinstance(timing, dict), f"Missing timing: {key}")
            require((timing.get("unit"), timing.get("warmupCalls"), timing.get("samples"),
                     timing.get("callsPerSample")) == ("ns/main call", 128, 20, 64),
                    f"Unexpected timing protocol: {key}")
            raw = timing.get("raw")
            require(isinstance(raw, list) and len(raw) == 20 and
                    all(type(x) in (int, float) and math.isfinite(x) and x >= 0 for x in raw),
                    f"Invalid raw samples: {key}")
            ordered = sorted(raw)
            require((timing.get("min"), timing.get("median"), timing.get("max")) ==
                    (ordered[0], (ordered[9] + ordered[10]) / 2, ordered[-1]),
                    f"Inconsistent timing summary: {key}")
        else:
            require("guestTiming" not in row, f"Verify-only record has timing: {key}")
        rows[key] = row
    require(len(machines) == 1, "Expected one Node machine record")
    require(set(rows) == {(level, name) for level in levels for name in NAMES},
            "Expected exactly 27 workloads per preset (26 timed, trap verify-only)")
    require(f"all {len(rows)} integer workloads passed" in text, "Missing guest success marker")
    for field in ("node", "v8", "platform", "arch", "osRelease", "cpu", "logicalCpus",
                  "totalMemoryBytes", "nodeFlags", "utc"):
        require(field in machines[0], f"Missing machine field: {field}")
    return rows, machines[0]


def summarize(out, level):
    verify, machine = parse_records((out / "verify.txt").read_text(encoding="utf-8"),
                                    "verify", LEVELS)
    guests = [parse_records((out / f"guest-{i}.txt").read_text(encoding="utf-8"),
                            "timing", (level,)) for i in (1, 2)]
    rows = []
    for name in NAMES:
        baseline = verify[("None", name)]
        candidates = [verify[(preset, name)] for preset in LEVELS]
        candidates += [records[(level, name)] for records, _ in guests]
        require(all(all(row[k] == baseline[k] for k in IDENTITY) for row in candidates),
                f"Source/module identity differs across presets or runs: {name}")
        rows.append({"name": name, **{k: baseline[k] for k in IDENTITY},
                     "guest_medians_ns": [records[(level, name)].get("guestTiming", {}).get("median")
                                          for records, _ in guests]})
    report = {"level": level, "notes": NOTE, "verify_records": len(verify),
              "guest_records_per_run": 27, "timed_records_per_run": 26,
              "all_presets_and_runs_identical": True,
              "machines": [machine] + [m for _, m in guests], "workloads": rows}
    write_json(out / "summary.json", report)
    lines = ["# Integer specialization baseline", "", f"Selected level: `{level}`.", "", NOTE,
             "", "108 verification records across four presets; each guest run has 26 timed",
             "workloads and one verify-only trap. Source/module identity matches across all runs.",
             "", "Hashes below are SHA-256. Guest medians are ns/main call; trap has no timing.",
             "", "| Name | Source bytes | Module bytes | Source hash | Module hash | Median 1 | Median 2 |",
             "| --- | ---: | ---: | --- | --- | ---: | ---: |"]
    for row in rows:
        medians = ["verify-only" if x is None else str(x) for x in row["guest_medians_ns"]]
        lines.append(f"| {row['name']} | {row['sourceBytes']} | {row['moduleBytes']} | "
                     f"{row['sourceSha256']} | {row['moduleSha256']} | {medians[0]} | {medians[1]} |")
    with (out / "summary.md").open("x", encoding="utf-8", newline="\n") as stream:
        stream.write("\n".join(lines) + "\n")


def main(argument):
    level = os.environ.get("INTEGER_BASELINE_LEVEL", "None")
    require(level in LEVELS, "INTEGER_BASELINE_LEVEL must be None/Basic/Standard/Aggressive")
    out = create_output(ROOT, argument)
    env = dict(os.environ, INTEGER_BASELINE_LEVEL=level, CARGO_NET_OFFLINE="true")
    write_json(out / "environment.json", {
        "utc": utc(), "recorder_command": sys.argv, "cwd": str(ROOT), "level": level,
        "level_environment_input": os.environ.get("INTEGER_BASELINE_LEVEL"),
        "python": sys.version, "notes": NOTE,
        "environment_overrides": {k: env[k] for k in ("INTEGER_BASELINE_LEVEL", "CARGO_NET_OFFLINE")},
    })
    before = hashes(ROOT)
    write_json(out / "hashes-before.json", before)
    success = False
    try:
        for name, command in (
            ("git-head", ["git", "--no-pager", "rev-parse", "HEAD"]),
            ("git-status", ["git", "--no-pager", "--no-optional-locks", "status", "--short"]),
            ("rustc", ["rustc", "-Vv"]), ("cargo", ["cargo", "--version"]),
            ("node", ["node", "--version"]),
        ):
            record(out, name, command, env)
        test = ["cargo", "test", "--offline", "-p", "was-compiler", "--test", "integer_baselines"]
        tail = ["--nocapture", "--test-threads=1"]
        record(out, "default-test", test + ["--"] + tail, env)
        record(out, "verify", test + ["execute_integer_workloads", "--", "--ignored", "--exact"] + tail, env)
        # Validate verification evidence before spending time on measurements.
        parse_records((out / "verify.txt").read_text(encoding="utf-8"), "verify", LEVELS)
        for i in (1, 2):
            record(out, f"compiler-{i}", ["cargo", "bench", "--offline", "-p", "was-compiler",
                   "--bench", "integer_specialization", "--", "--sample-count", "100",
                   "--sample-size", "1", "--timer", "os", "--color", "never"], env)
        for i in (1, 2):
            record(out, f"guest-{i}", test + ["record_integer_guest_timing", "--", "--ignored",
                   "--exact"] + tail, env)
            parse_records((out / f"guest-{i}.txt").read_text(encoding="utf-8"), "timing", (level,))
        success = True
    finally:
        after = hashes(ROOT)
        write_json(out / "hashes-after.json", after)
        stable = before == after
        write_json(out / "stability.json", {"utc_finished": utc(), "commands_completed": success,
                                           "inputs_unchanged": stable})
        require(stable, "Inputs changed during recording; evidence is invalid")
    summarize(out, level)
    print("Baseline complete:", out, flush=True)


def self_test():
    import tempfile
    import unittest
    from unittest.mock import patch

    def fixture(mode, levels):
        lines = [json.dumps(dict.fromkeys(("node", "v8", "platform", "arch", "osRelease", "cpu",
                 "logicalCpus", "totalMemoryBytes", "nodeFlags", "utc"), "test") | {"mode": mode})]
        for level in levels:
            for name in NAMES:
                row = {"level": level, "name": name, "sourceBytes": 1, "moduleBytes": 2,
                       "sourceSha256": "a" * 64, "moduleSha256": "b" * 64}
                if mode == "timing" and name != "trap_order":
                    row["guestTiming"] = {"unit": "ns/main call", "warmupCalls": 128, "samples": 20,
                                          "callsPerSample": 64, "raw": list(range(20)),
                                          "min": 0, "median": 9.5, "max": 19}
                lines.append(json.dumps(row))
        return "test example ... " + "\n".join(lines) + f"\nall {27 * len(levels)} integer workloads passed\n"

    class Tests(unittest.TestCase):
        def test_parse_and_rejections(self):
            text = fixture("timing", ("None",))
            self.assertEqual(len(parse_records(text, "timing", ("None",))[0]), 27)
            self.assertEqual(len(parse_records(fixture("verify", LEVELS), "verify", LEVELS)[0]), 108)
            for broken in (text + text.splitlines()[1], text.replace('"median": 9.5', '"median": 8'),
                           text.replace('"name": "trap_order"', '"name": "other"')):
                with self.assertRaises(ValueError):
                    parse_records(broken, "timing", ("None",))

        def test_output_safety(self):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                with patch.object(Path, "cwd", return_value=root):
                    for bad in (".", "docs/benchmarks", "../escape", "docs/benchmarks/../escape"):
                        with self.assertRaises(ValueError):
                            create_output(root, bad)
                    out = create_output(root, "docs/benchmarks/new")
                    with self.assertRaises(FileExistsError):
                        create_output(root, "docs/benchmarks/new")
                    self.assertEqual(list(out.iterdir()), [])
                with self.assertRaises(ValueError):
                    create_output(root, "docs/benchmarks/wrong-cwd")

        def test_summary_and_parity(self):
            with tempfile.TemporaryDirectory() as directory:
                out = Path(directory)
                (out / "verify.txt").write_text(fixture("verify", LEVELS), encoding="utf-8")
                for i in (1, 2):
                    (out / f"guest-{i}.txt").write_text(fixture("timing", ("None",)), encoding="utf-8")
                summarize(out, "None")
                self.assertEqual(len(json.loads((out / "summary.json").read_text())["workloads"]), 27)
                path = out / "guest-2.txt"
                path.write_text(path.read_text().replace("b" * 64, "c" * 64))
                with self.assertRaises(ValueError):
                    summarize(out, "None")

        def test_failed_commands_preserve_output(self):
            for failure in (subprocess.CompletedProcess(["test"], 7),
                            subprocess.TimeoutExpired(["test"], 120), OSError("missing tool")):
                with tempfile.TemporaryDirectory() as directory:
                    out = Path(directory)
                    def run(command, **kwargs):
                        kwargs["stdout"].write(b"partial raw\xff\n")
                        if isinstance(failure, Exception):
                            raise failure
                        return failure
                    with patch.object(subprocess, "run", side_effect=run):
                        with self.assertRaises(RuntimeError):
                            record(out, "failure", ["test"], {})
                    self.assertEqual((out / "failure.txt").read_bytes(), b"partial raw\xff\n")
                    metadata = json.loads((out / "failure.json").read_text())
                    self.assertEqual(metadata["timed_out"], isinstance(failure, subprocess.TimeoutExpired))

    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(self_test())
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    try:
        main(sys.argv[1])
    except (ValueError, RuntimeError, OSError) as error:
        sys.exit(str(error))
