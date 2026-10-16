'use strict';

const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const mode = process.argv[2];
assert.ok(['verify', 'marker', 'capacity', 'timing', 'layout'].includes(mode));
const cases = JSON.parse(fs.readFileSync(0, 'utf8'));
console.log(JSON.stringify({
    mode, utc: new Date().toISOString(), node: process.version, v8: process.versions.v8,
    platform: process.platform, arch: process.arch, osRelease: os.release(),
    cpu: os.cpus()[0].model, logicalCpus: os.cpus().length,
    totalMemoryBytes: os.totalmem(), nodeFlags: process.execArgv,
}));

if (mode === 'layout') {
    assert.deepEqual(cases.map(c => c[0]), ['number', 'bool', 'string']);
    for (const [element, , bytes] of cases) {
        const e = new WebAssembly.Instance(new WebAssembly.Module(new Uint8Array(bytes))).exports;
        const handle = e.main() >>> 0;
        const view = new DataView(e.memory.buffer);
        assert.equal(handle % 8, 0);
        const data = view.getUint32(handle, true);
        assert.equal(data, handle + 8);
        assert.equal(view.getUint32(handle + 4, true), 3);
        if (element === 'number') {
            assert.equal(data % 8, 0);
            assert.deepEqual([0, 8, 16].map(p => view.getFloat64(data + p, true)), [1, 0, 1]);
        } else if (element === 'bool') {
            assert.deepEqual([...new Uint8Array(e.memory.buffer, data, 3)], [1, 0, 1]);
        } else {
            assert.equal(data % 4, 0);
            const handles = [0, 4, 8].map(p => view.getUint32(data + p, true));
            const text = handles.map(h => {
                const p = view.getUint32(h, true), n = view.getUint32(h + 4, true);
                return new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array(e.memory.buffer, p, n));
            });
            assert.deepEqual(text, ['é', 'x', 'é']);
            assert.equal(handles[0], handles[2], 'interned strings share an immutable payload');
        }
    }
    console.log('typed-array measurements passed');
    process.exit(0);
}

// Worked ABI examples, independent of production constants and emitted code:
// an 8-byte header plus N slots, total rounded up to 8. Empty still reserves 8.
const lengths = [0, 1, 2, 3, 7, 8, 9];
const reservations = {
    number: [8, 16, 24, 32, 64, 72, 80],
    bool: [8, 16, 16, 16, 16, 16, 24],
    string: [8, 16, 16, 24, 40, 40, 48],
};
const extra = {
    number: { page: 589824, pages: 10, alias: 32, four: 40, shortPair: 88, pair: 144 },
    bool: { page: 131072, pages: 3, alias: 16, four: 16, shortPair: 32, pair: 32 },
    string: { page: 327680, pages: 6, alias: 24, four: 24, shortPair: 56, pair: 80 },
};
const expected = {};
for (const element of Object.keys(reservations)) {
    lengths.forEach((n, i) => {
        expected[`${element}_construct_${n}`] = [128 * n, 128 * reservations[element][i]];
    });
    const e = extra[element];
    expected[`${element}_page_8`] = [65536, e.page];
    expected[`${element}_alias_replace`] = [512, e.alias];
    // Two initial on values and the fourth element changed before its live yield.
    expected[`${element}_iteration_live`] = [384, e.four];
    expected[`${element}_equality_length`] = [0, e.shortPair];
    expected[`${element}_equality_early`] = [0, e.pair];
    expected[`${element}_equality_late`] = [0, e.pair];
    expected[`${element}_equality_equal`] = [128, e.pair];
    // All three singleton arrays reserve 16; two cells + two environment slots = 32.
    expected[`${element}_capture_shared`] = [128, 48];
    // 512 fresh iteration cells/environments at 16 bytes, plus one four-slot array.
    expected[`${element}_capture_iteration`] = [256, 8192 + e.four];
}
assert.deepEqual(cases.map(c => c[0]).sort(), Object.keys(expected).sort());
const hash = bytes => crypto.createHash('sha256').update(Buffer.from(bytes)).digest('hex');
const pages = e => e.memory.buffer.byteLength / 65536;

for (const [name, source, bytes] of cases) {
    const module = new WebAssembly.Module(new Uint8Array(bytes));
    const fresh = () => new WebAssembly.Instance(module).exports;
    const [result, bump] = expected[name];
    const record = {
        name, result, sourceBytes: source.length, moduleBytes: bytes.length,
        sourceSha256: hash(source), moduleSha256: hash(bytes),
    };
    if (mode === 'marker' || mode === 'verify') {
        const marked = fresh();
        const before = marked.__alloc(0) >>> 0;
        assert.equal(marked.main(), result, name);
        const after = marked.__alloc(0) >>> 0;
        assert.equal(before % 8, 0);
        assert.equal(after % 8, 0);
        record.alignedBumpBytes = after - before - 8;
        assert.equal(record.alignedBumpBytes, bump, name);
    }
    if (mode === 'capacity' || mode === 'verify') {
        // This instance never receives markers or timing instrumentation.
        const capacity = fresh();
        record.initialPages = pages(capacity);
        assert.equal(capacity.main(), result, name);
        record.finalPages = pages(capacity);
        assert.equal(record.initialPages, 1);
        assert.equal(record.finalPages, name.endsWith('_page_8') ? extra[name.split('_')[0]].pages : 1);
    }
    if (mode === 'timing') {
        // Match M0's finite timing protocol without modifying the M0 harness.
        // No compilation, instantiation, checks, markers or page queries in timing.
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
console.log('typed-array measurements passed');
