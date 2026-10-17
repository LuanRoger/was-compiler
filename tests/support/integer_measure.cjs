'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const crypto = require('node:crypto');
const os = require('node:os');
const mode = process.argv[2];
assert.ok(['verify', 'timing'].includes(mode));
assert.ok(Number(process.versions.node.split('.')[0]) >= 24, 'Node.js 24+ required');
const cases = JSON.parse(fs.readFileSync(0, 'utf8'));
// Worked results, not an implementation of the proposed range/proof analysis:
// each positive block contributes 143, 3, or -143; 32 blocks yield the literals below.
const expected = {
    literal_1: 143, locals_1: 143, compare_1: 3, negate_1: -143,
    literal_32: 4576, locals_32: 4576, compare_32: 96, negate_32: -4576,
    loop_local: 4576, fraction: 3.75, negative_zero: -0, negate_zero: -0,
    positive_zero: 0, overflow: 2147483649, underflow_i32: -2147483650,
    negate_min: 2147483648, unknown: 8.5, mutable: 3.5, capture: 144,
    multiply: 63, divide: 3.5, remainder: -1, nan: NaN, infinity: Infinity,
    subnormal: Number.MIN_VALUE, effects: -6, trap_order: undefined,
};
const levels = [...new Set(cases.map(c => c[1]))];
for (const level of levels) {
    assert.deepEqual(cases.filter(c => c[1] === level).map(c => c[0]).sort(), Object.keys(expected).sort());
}
const hash = bytes => crypto.createHash('sha256').update(Buffer.from(bytes)).digest('hex');
console.log(JSON.stringify({mode, utc: new Date().toISOString(), node: process.version,
    v8: process.versions.v8, platform: process.platform, arch: process.arch,
    osRelease: os.release(), cpu: os.cpus()[0].model, logicalCpus: os.cpus().length,
    totalMemoryBytes: os.totalmem(), nodeFlags: process.execArgv}));
for (const [name, level, source, bytes] of cases) {
    const module = new WebAssembly.Module(new Uint8Array(bytes));
    const fresh = () => {
        const effects = [];
        const instance = new WebAssembly.Instance(module, {env: {mark(n) { effects.push(n); return n; }}});
        return {main: instance.exports.main, effects};
    };
    const guest = fresh();
    if (name === 'trap_order') {
        assert.throws(() => guest.main(7.5), WebAssembly.RuntimeError);
        assert.deepEqual(guest.effects, [1], 'bounds trap precedes RHS effect');
    } else {
        assert.ok(Object.is(guest.main(7.5), expected[name]), `${level}/${name}`);
        assert.deepEqual(guest.effects, name === 'effects' ? [3, 9] : []);
    }
    if (name === 'unknown') {
        for (const [input, result] of [[-0, 1], [2147483647, 2147483648], [Infinity, Infinity], [-Infinity, -Infinity], [NaN, NaN], [Number.MIN_VALUE, 1]]) {
            assert.ok(Object.is(fresh().main(input), result), `unknown input ${input}`);
        }
    }
    const record = {name, level, sourceBytes: source.length, moduleBytes: bytes.length,
        sourceSha256: hash(source), moduleSha256: hash(bytes)};
    if (mode === 'timing' && name !== 'trap_order') {
        // Keep compilation, instantiation, warmup, assertions, and reporting outside timing.
        const warm = fresh();
        for (let i = 0; i < 128; i++) warm.main(7.5);
        const raw = [];
        for (let sample = 0; sample < 20; sample++) {
            const current = fresh();
            let last;
            const start = process.hrtime.bigint();
            for (let call = 0; call < 64; call++) last = current.main(7.5);
            const elapsed = process.hrtime.bigint() - start;
            assert.ok(Object.is(last, expected[name]), name);
            assert.deepEqual(current.effects, name === 'effects' ? Array.from({length: 64}, () => [3, 9]).flat() : []);
            raw.push(Number(elapsed) / 64);
        }
        const sorted = [...raw].sort((a, b) => a - b);
        record.guestTiming = {unit: 'ns/main call', warmupCalls: 128, samples: 20,
            callsPerSample: 64, min: sorted[0], median: (sorted[9] + sorted[10]) / 2,
            max: sorted[19], raw};
    }
    console.log(JSON.stringify(record));
}
console.log(`all ${cases.length} integer workloads passed`);
