"""Offline, bounded M4 recorder. Run from repo root with a NEW evidence directory.

Only writes under docs/benchmarks; never overwrites a record. Uses stdlib only.
Measurements run sequentially, independently of validation and report generation.
"""

import datetime
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / "docs/benchmarks"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def manifest(paths):
    return {p.relative_to(ROOT).as_posix(): digest(p) for p in sorted(set(paths))}


def production_manifest():
    paths = list(ROOT.glob("crates/*/src/**/*.rs"))
    paths += list(ROOT.glob("runtime/*.cjs"))
    paths += [ROOT / "Cargo.toml"]
    paths += list(ROOT.glob("crates/*/Cargo.toml"))
    paths += [ROOT / "Cargo.lock"]
    return manifest(paths)


def command_output(command):
    result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                            errors="replace", timeout=120, check=False)
    return result.returncode, result.stdout


def record(out, name, command):
    print("COMMAND:", " ".join(command), flush=True)
    start = datetime.datetime.now(datetime.timezone.utc).isoformat()
    code, output = command_output(command)
    (out / name).write_text(f"UTC: {start}\nCOMMAND: {' '.join(command)}\nEXIT: {code}\n{output}",
                            encoding="utf-8")
    if code:
        raise RuntimeError(f"{name}: exit {code}; see preserved output")


def records(path):
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        # libtest prefixes the first stdout line with its test name.
        start = line.find("{")
        if start >= 0:
            try:
                rows.append(json.loads(line[start:]))
            except json.JSONDecodeError:
                pass
    return {r["name"]: r for r in rows if "name" in r}


def compiler_records(path):
    timings, sizes = {}, {}
    stage = None
    for line in path.read_text(encoding="utf-8").splitlines():
        header = re.match(r"[├╰]─ (\w+)\s+│", line)
        if header:
            stage = header[1]
        row = re.search(r"[├╰]─ (\w+_\d+)\s+", line)
        if row and stage:
            median = line.split("│")[-4].strip()
            value, unit = median.split()
            timings[f"{stage}/{row[1]}"] = float(value) * {"ns": 1, "µs": 1000, "ms": 1000000}[unit]
        size = re.fullmatch(r"(\w+_\d+),(\d+),(\d+)", line)
        if size:
            sizes[size[1]] = [int(size[2]), int(size[3])]
    assert len(timings) == 45 and len(sizes) == 9, path
    return timings, sizes


def comparisons(out):
    old = records(BASE / "2026-10-15-m0-markers.txt")
    old_pages = records(BASE / "2026-10-15-m0-capacity.txt")
    old_guest = [records(BASE / f"2026-10-15-m0-guest-{i}.txt") for i in (1, 2)]
    new = records(out / "numeric-markers.txt")
    new_pages = records(out / "numeric-capacity.txt")
    new_guest = [records(out / f"numeric-guest-{i}.txt") for i in (1, 2)]
    assert len(old) == len(new) == 17 and old.keys() == new.keys()
    report = {"numeric": {}, "typed": {}, "compiler": {}}
    md = ["# M4 machine-generated comparison tables", "",
          "Raw command output and environment/source manifests are adjacent. Timings are observations, not speedup claims.",
          "", "## Unchanged M0 numeric corpus", "",
          "Pages show initial/final capacity, not live bytes. Source/module identity uses SHA-256.", "",
          "| Case | Source identical | Module identical | Module bytes M0/M4 | Bump bytes M0/M4 | Pages M0 / M4 |",
          "| --- | --- | --- | ---: | ---: | --- |"]
    for name, before in old.items():
        after = new[name]
        identity = {k: before[k] == after[k] for k in ("sourceSha256", "moduleSha256")}
        for runs in [new_pages, *new_guest]:
            assert all(runs[name][k] == after[k] for k in ("sourceSha256", "moduleSha256"))
        row = {"before": before, "after": after, "identity": identity,
               "pages_before": [old_pages[name][k] for k in ("initialPages", "finalPages")],
               "pages_after": [new_pages[name][k] for k in ("initialPages", "finalPages")],
               "guest_medians_before_ns": [r[name]["guestTiming"]["median"] for r in old_guest],
               "guest_medians_after_ns": [r[name]["guestTiming"]["median"] for r in new_guest]}
        report["numeric"][name] = row
        md.append(f"| {name} | {identity['sourceSha256']} | {identity['moduleSha256']} | "
                  f"{before['moduleBytes']}/{after['moduleBytes']} | "
                  f"{before['alignedBumpBytes']}/{after['alignedBumpBytes']} | "
                  f"{row['pages_before']} / {row['pages_after']} |")
    md += ["", "### Guest medians (ns/main call)", "",
           "| Case | M0 run 1 / 2 | M4 run 1 / 2 |", "| --- | ---: | ---: |"]
    for name, row in report["numeric"].items():
        md.append(f"| {name} | {' / '.join(map(str, row['guest_medians_before_ns']))} | "
                  f"{' / '.join(map(str, row['guest_medians_after_ns']))} |")

    typed = records(out / "typed-markers.txt")
    typed_pages = records(out / "typed-capacity.txt")
    typed_guest = [records(out / f"typed-guest-{i}.txt") for i in (1, 2)]
    assert len(typed) == 48
    md += ["", "## New matched type-width controls (no historical new-type baseline)", "",
           "| Case | Source bytes | Module bytes | Bump bytes | Pages initial/final | Guest median ns run 1 / 2 |",
           "| --- | ---: | ---: | ---: | --- | ---: |"]
    for name, row in typed.items():
        for runs in [typed_pages, *typed_guest]:
            assert all(runs[name][k] == row[k] for k in ("sourceSha256", "moduleSha256"))
        pages = [typed_pages[name][k] for k in ("initialPages", "finalPages")]
        medians = [r[name]["guestTiming"]["median"] for r in typed_guest]
        report["typed"][name] = {**row, "pages": pages, "guest_medians_ns": medians}
        md.append(f"| {name} | {row['sourceBytes']} | {row['moduleBytes']} | {row['alignedBumpBytes']} | "
                  f"{pages} | {' / '.join(map(str, medians))} |")

    old_compiler, old_sizes = compiler_records(BASE / "2026-10-15-m0-compiler.txt")
    current = [compiler_records(out / f"compiler-{i}.txt") for i in (1, 2)]
    assert all(sizes == old_sizes for _, sizes in current), "legacy compiler corpus size drift"
    md += ["", "## Unchanged compiler harness (median ns; 100 samples, sample size 1)", "",
           "Only one historical M0 compiler run exists; no repeated historical compiler comparison is possible.",
           "All nine source/module byte sizes are unchanged. Guest timing is excluded.", "",
           "| Stage/case | M0 | M4 run 1 | M4 run 2 |", "| --- | ---: | ---: | ---: |"]
    for name, median in old_compiler.items():
        medians = [rows[name] for rows, _ in current]
        report["compiler"][name] = {"m0_median_ns": median, "m4_medians_ns": medians}
        md.append(f"| {name} | {median} | {medians[0]} | {medians[1]} |")
    report["compiler_source_module_sizes"] = old_sizes
    (out / "comparison.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    (out / "comparison.md").write_text("\n".join(md) + "\n", encoding="utf-8")


def main():
    if len(sys.argv) != 2 or Path.cwd() != ROOT:
        raise SystemExit("Run from repo root: python tests/support/record_memory_m4.py docs/benchmarks/NEW-DIRECTORY")
    out = (ROOT / sys.argv[1]).resolve()
    if not out.is_relative_to(BASE) or out == BASE:
        raise SystemExit("Evidence must be in a new subdirectory of docs/benchmarks")
    out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()):
        raise SystemExit("Refusing to overwrite evidence; choose an empty directory")
    original = ["tests/support/memory_corpus.rs", "tests/support/memory_measure.cjs",
                "crates/compiler/tests/memory_benchmarks.rs", "tests/support/process.rs",
                "crates/compiler/benches/compiler.rs", "crates/compiler/benches/support/corpus.rs"]
    baseline_hashes = dict((p, h) for h, p in re.findall(
        r"^SHA256 ([a-f0-9]+)  (.+)$",
        (BASE / "2026-10-15-m0-environment.txt").read_text(encoding="utf-8"), re.M))
    for path in original:
        assert digest(ROOT / path) == baseline_hashes[path], f"M0 input changed: {path}"
    historical = manifest(BASE.glob("2026-10-15-m0-*.txt"))
    production = production_manifest()
    measured = manifest([ROOT / p for p in original] + [
        ROOT / "tests/support/typed_array_measure_corpus.rs",
        ROOT / "tests/support/typed_array_measure.cjs",
        ROOT / "tests/support/record_memory_m4.py",
        ROOT / "crates/compiler/tests/typed_array_measurements.rs"])
    env_commands = [["git", "--no-pager", "rev-parse", "HEAD"],
                    ["git", "--no-optional-locks", "status", "--short"],
                    ["rustc", "--version", "--verbose"], ["cargo", "--version"],
                    ["node", "--version"], ["python", "--version"],
                    ["node", "-p", "JSON.stringify({cpu:require('node:os').cpus()[0].model,logicalCpus:require('node:os').cpus().length,os:require('node:os').release(),arch:process.arch,node:process.version,v8:process.versions.v8,totalMemoryBytes:require('node:os').totalmem()})"]]
    environment = {
        "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "recorder_command": " ".join(sys.argv), "cwd": str(ROOT),
        "notes": "Sequential offline runs; debug Rust test caller, default guest options; optimized compiler bench. Power/thermal/background activity uncontrolled. Not release validation.",
        "commands": [{"command": c, "exit": code, "output": text}
                     for c in env_commands for code, text in [command_output(c)]],
        "m0_inputs_unchanged": True, "historical_evidence_sha256": historical,
        "measurement_files_sha256": measured, "production_files_sha256": production,
    }
    (out / "environment.json").write_text(json.dumps(environment, indent=2) + "\n", encoding="utf-8")
    for label, target, prefix in [("numeric", "memory_benchmarks", "record_memory"),
                                   ("typed", "typed_array_measurements", "record_typed_array")]:
        for suffix, test in [("markers", "markers"), ("capacity", "capacity"),
                             ("guest-1", "guest_timing"), ("guest-2", "guest_timing")]:
            record(out, f"{label}-{suffix}.txt", ["cargo", "test", "--offline", "-p", "was-compiler",
                "--test", target, f"{prefix}_{test}", "--", "--ignored", "--exact", "--nocapture", "--test-threads=1"])
    compiler = ["cargo", "bench", "--offline", "-p", "was-compiler", "--bench", "compiler", "--",
                "--sample-count", "100", "--sample-size", "1", "--timer", "os", "--color", "never"]
    for i in (1, 2):
        record(out, f"compiler-{i}.txt", compiler)
    assert production_manifest() == production, "production changed while measuring; discard comparison"
    assert manifest(BASE.glob("2026-10-15-m0-*.txt")) == historical, "historical evidence changed"
    assert all(digest(ROOT / p) == h for p, h in measured.items()), "measurement inputs changed"
    comparisons(out)
    (out / "stability.json").write_text(json.dumps({"production_unchanged_during_runs": True,
        "measurement_files_unchanged": True, "m0_evidence_unchanged": True,
        "utc_finished": datetime.datetime.now(datetime.timezone.utc).isoformat()}, indent=2) + "\n", encoding="utf-8")
    print("Comparisons complete:", out, flush=True)


if __name__ == "__main__":
    main()
