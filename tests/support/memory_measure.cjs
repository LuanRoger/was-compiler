'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const crypto = require('node:crypto');
const cases = JSON.parse(fs.readFileSync(0, 'utf8'));
const mode = process.argv[2];
assert.ok(['verify', 'marker', 'capacity', 'timing'].includes(mode), 'known measurement mode');
const os = require('node:os');
console.log(JSON.stringify({
    mode, utc: new Date().toISOString(), node: process.version, v8: process.versions.v8,
    platform: process.platform, arch: process.arch, osRelease: os.release(),
    cpu: os.cpus()[0].model, logicalCpus: os.cpus().length,
    totalMemoryBytes: os.totalmem(), nodeFlags: process.execArgv,
}));

// Worked M0 expectations: [result per main, aligned bytes consumed per main].
// Numeric arrays reserve 8 descriptor bytes + 8 per element; empty reserves 8.
// Shared cells/environment slots each reserve 8. Static strings consume no bump.
const expected = {
    construct_0: [0, 1024],
    construct_1: [128, 2048],
    construct_7: [896, 8192],
    construct_8: [1024, 9216],
    construct_9: [1152, 10240],
    page_crossing: [8192, 131072],
    alias_index_replace: [1536, 32],
    iteration_live_alias: [1792, 40],
    equality_length: [0, 88],
    equality_early: [0, 144],
    equality_late: [0, 144],
    equality_equal: [128, 144],
    string_7_equal: [128, 0],
    string_8_early: [0, 0],
    string_9_late: [0, 0],
    capture_shared: [8256, 48],
    capture_iteration: [1280, 8232],
};
assert.deepEqual(cases.map(c => c[0]).sort(), Object.keys(expected).sort());
const hash = bytes => crypto.createHash('sha256').update(Buffer.from(bytes)).digest('hex');
const pages = e => e.memory.buffer.byteLength / 65536;

for (const [name, source, bytes] of cases) {
    const module = new WebAssembly.Module(new Uint8Array(bytes));
    const fresh = () => new WebAssembly.Instance(module).exports;
    const [result, consumed] = expected[name];

    const record = {
        name, result, sourceBytes: source.length, moduleBytes: bytes.length,
        sourceSha256: hash(source), moduleSha256: hash(bytes),
    };
    if (mode === 'marker' || mode === 'verify') {
        // Markers perturb allocation and can grow memory. Never reuse this
        // instance for capacity/timing; subtract the first marker's reservation.
        const marked = fresh();
        const before = marked.__alloc(0) >>> 0;
        assert.equal(marked.main(), result, name);
        const after = marked.__alloc(0) >>> 0;
        assert.equal(before % 8, 0);
        assert.equal(after % 8, 0);
        record.alignedBumpBytes = after - before - 8;
        assert.equal(record.alignedBumpBytes, consumed, name);
    }
    if (mode === 'capacity' || mode === 'verify') {
        // Capacity, not live bytes or RSS, on a fresh unmarked instance.
        const capacity = fresh();
        record.initialPages = pages(capacity);
        assert.equal(capacity.main(), result, name);
        record.finalPages = pages(capacity);
        assert.ok(record.finalPages >= record.initialPages);
        if (name === 'page_crossing') {
            assert.equal(record.initialPages, 1);
            assert.equal(record.finalPages, 3);
        }
    }
    if (mode === 'timing') {
        // Compilation, instantiation, warmup, correctness and reporting are not
        // timed. Each sample has fresh allocation state; module/JIT is shared.
        const warm = fresh();
        for (let i = 0; i < 8; i++) assert.equal(warm.main(), result, name);
        const samples = [];
        for (let sample = 0; sample < 20; sample++) {
            const guest = fresh();
            let checksum = 0;
            const start = process.hrtime.bigint();
            for (let call = 0; call < 4; call++) checksum += guest.main();
            const elapsed = process.hrtime.bigint() - start;
            assert.equal(checksum, 4 * result, name);
            samples.push(Number(elapsed) / 4);
        }
        const sorted = [...samples].sort((a, b) => a - b);
        record.guestTiming = {
            unit: 'ns/main call', warmupCalls: 8, samples: 20, callsPerSample: 4,
            min: sorted[0], median: (sorted[9] + sorted[10]) / 2,
            max: sorted[19], raw: samples,
        };
    }
    console.log(JSON.stringify(record));
}

// Public raw allocation boundary, on its own instance, not a workload sample.
if (mode === 'marker' || mode === 'verify') {
    const allocator = new WebAssembly.Instance(
        new WebAssembly.Module(new Uint8Array(cases[0][2]))).exports;
    const offsets = [0, 1, 7, 8, 9, 0].map(n => allocator.__alloc(n) >>> 0);
    assert.deepEqual(offsets.slice(1).map((p, i) => p - offsets[i]), [8, 8, 8, 8, 16]);
    console.log(JSON.stringify({ rawAllocationRequests: [0, 1, 7, 8, 9],
        alignedReservations: [8, 8, 8, 8, 16], offsets }));
}
console.log('all 17 memory workloads passed');
