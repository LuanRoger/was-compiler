#[path = "support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{Compiler, OptimizationLevel, Type};
use wasmparser::Validator;

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const MEMBERSHIP_SOURCE: &str = r#"
    fn exclusive(n: number, a: number, b: number, s: number) -> bool {
        return n in a..b by s;
    }
    fn inclusive(n: number, a: number, b: number, s: number) -> bool {
        return n in a..=b by s;
    }
    fn defaultExclusive(n: number, a: number, b: number) -> bool {
        return n in a..b;
    }
    fn defaultInclusive(n: number, a: number, b: number) -> bool {
        return n in a..=b;
    }
"#;

const EFFECTS_SOURCE: &str = r#"
    fn main(n: number, a: number, b: number, s: number) -> bool {
        return sample(1, n) in sample(2, a)..=sample(3, b) by sample(4, s);
    }
    fn skipAnd() -> bool {
        return false && (sample(1, 2) in sample(2, 0)..sample(3, 4) by sample(4, 0));
    }
    fn skipOr() -> bool {
        return true || (sample(1, 2) in sample(2, 0)..sample(3, 4) by sample(4, 0));
    }
    fn skipIf() -> bool {
        return if false { sample(1, 2) in sample(2, 0)..sample(3, 4) by sample(4, 0) }
               else { true };
    }
    fn abruptCandidate() -> bool {
        return { sample(1, 2); return true; 2 }
            in sample(2, 0)..sample(3, 4) by sample(4, 1);
    }
    fn abruptStart() -> bool {
        return sample(1, 2) in { sample(2, 0); return false; 0 }
            ..sample(3, 4) by sample(4, 1);
    }
    fn abruptEnd() -> bool {
        return sample(1, 2) in sample(2, 0)
            ..{ sample(3, 4); return true; 4 } by sample(4, 1);
    }
"#;

const CORRUPTION_SOURCE: &str = r#"
    fn main(n: number) -> bool {
        let r = 1..=4 by 2;
        mutate();
        return n in r;
    }
"#;

fn effects_compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function(
            "sample",
            "env",
            "sample",
            vec![Type::Number, Type::Number],
            Type::Number,
        )
        .expect("register effect probe");
    compiler
}

fn corruption_compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("mutate", "env", "mutate", vec![], Type::Void)
        .expect("register record mutation probe");
    compiler
}

fn compile(compiler: &Compiler, source: &str) -> Vec<u8> {
    let bytes = compiler
        .compile_source(source)
        .expect("compile membership through public API");
    Validator::new()
        .validate_all(&bytes)
        .expect("valid membership Wasm");
    bytes
}

fn run_node_at_all_levels(mut compiler: Compiler, source: &str, script: &str) {
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compile(&compiler, source);
        let output = test_process::output(
            Command::new("node").arg("-e").arg(script),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{level:?}: Node failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn public_membership_fixtures_validate_at_every_optimization_level() {
    for (mut compiler, source) in [
        (Compiler::new(), MEMBERSHIP_SOURCE),
        (effects_compiler(), EFFECTS_SOURCE),
        (corruption_compiler(), CORRUPTION_SOURCE),
    ] {
        let baseline = compile(&compiler, source);
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(compile(&compiler, source), baseline, "{level:?}");
        }
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_exact_membership_uses_literal_interval_and_congruence_expectations() {
    let script = r#"
        const assert = require('node:assert/strict');
        const e = new WebAssembly.Instance(
            new WebAssembly.Module(require('node:fs').readFileSync(0))).exports;
        // Literal answers, not a reproduction of the guest membership algorithm.
        for (const [name, bounds, cases] of [
            ['exclusive', [1,7,2], [[0,0],[1,1],[2,0],[3,1],[5,1],[6,0],[7,0],[8,0]]],
            ['inclusive', [1,7,2], [[0,0],[1,1],[2,0],[3,1],[5,1],[6,0],[7,1],[8,0]]],
            ['inclusive', [1,6,2], [[1,1],[5,1],[6,0]]],
            ['defaultExclusive', [1,4], [[0,0],[1,1],[2,1],[3,1],[4,0]]],
            ['defaultInclusive', [1,4], [[0,0],[1,1],[2,1],[3,1],[4,1],[5,0]]],
            ['defaultExclusive', [4,1], [[0,0],[1,0],[2,0],[3,0],[4,0],[5,0]]],
            ['defaultInclusive', [4,1], [[0,0],[1,0],[2,0],[3,0],[4,0],[5,0]]],
            ['exclusive', [7,1,-2], [[8,0],[7,1],[6,0],[5,1],[3,1],[2,0],[1,0],[0,0]]],
            ['inclusive', [7,1,-2], [[8,0],[7,1],[6,0],[5,1],[3,1],[2,0],[1,1],[0,0]]],
            ['inclusive', [7,2,-2], [[7,1],[3,1],[2,0]]],
            ['inclusive', [1,7,-2], [[1,0],[3,0],[7,0]]],
            ['inclusive', [7,1,2], [[1,0],[3,0],[7,0]]],
            ['exclusive', [5,5,2], [[4,0],[5,0],[6,0]]],
            ['inclusive', [5,5,2], [[4,0],[5,1],[6,0]]],
            ['exclusive', [5,5,-2], [[4,0],[5,0],[6,0]]],
            ['inclusive', [5,5,-2], [[4,0],[5,1],[6,0]]],
            ['defaultExclusive', [5,5], [[4,0],[5,0],[6,0]]],
            ['defaultInclusive', [5,5], [[4,0],[5,1],[6,0]]],
            ['exclusive', [-0,0,1], [[-0,0],[0,0]]],
            ['inclusive', [-0,0,1], [[-0,1],[0,1],[-1,0],[1,0]]],
            ['inclusive', [0,-0,-1], [[-0,1],[0,1],[-1,0],[1,0]]],
            ['exclusive', [-0,2,1], [[-0,1],[0,1],[1,1],[2,0]]],
            ['exclusive', [0,-2,-1], [[-0,1],[0,1],[-1,1],[-2,0]]],
            ['inclusive', [-2,-0,1], [[-2,1],[-1,1],[-0,1],[0,1],[1,0]]],
            ['inclusive', [2,0,-1], [[2,1],[1,1],[-0,1],[0,1],[-1,0]]],
            ['inclusive', [-9007199254740991,9007199254740991,9007199254740991],
                [[-9007199254740991,1],[0,1],[9007199254740991,1],[-1,0],[1,0],[9007199254740990,0]]],
            ['exclusive', [-9007199254740991,9007199254740991,9007199254740991],
                [[-9007199254740991,1],[0,1],[9007199254740991,0]]],
            ['inclusive', [9007199254740991,-9007199254740991,-9007199254740991],
                [[9007199254740991,1],[0,1],[-9007199254740991,1],[-1,0],[1,0]]],
            ['exclusive', [9007199254740991,-9007199254740991,-9007199254740991],
                [[9007199254740991,1],[0,1],[-9007199254740991,0]]],
            ['inclusive', [-9007199254740991,9007199254740991,3],
                [[-9007199254740991,1],[-9007199254740988,1],[9007199254740989,1],
                 [9007199254740990,0],[9007199254740991,0]]],
            ['inclusive', [9007199254740991,-9007199254740991,-3],
                [[9007199254740991,1],[9007199254740988,1],[-9007199254740989,1],
                 [-9007199254740990,0],[-9007199254740991,0]]],
            ['inclusive', [-9007199254740991,9007199254740991,9007199254740990],
                [[-9007199254740991,1],[-1,1],[9007199254740989,1],[9007199254740990,0],[9007199254740991,0]]],
            ['inclusive', [9007199254740991,-9007199254740991,-9007199254740990],
                [[9007199254740991,1],[1,1],[-9007199254740989,1],[-9007199254740990,0],[-9007199254740991,0]]],
        ]) {
            for (const [n, expected] of cases) {
                assert.equal(e[name](n,...bounds),expected,`${name}(${n}; ${bounds})`);
            }
        }
        const invalid = [NaN, Infinity, -Infinity, 0.5, -0.5, Number.MIN_VALUE,
            -Number.MIN_VALUE, 9007199254740992, -9007199254740992,
            Number.MAX_VALUE, -Number.MAX_VALUE];
        for (const n of invalid) {
            for (const name of ['exclusive','inclusive']) {
                assert.equal(e[name](n,-9007199254740991,9007199254740991,1),0,
                    `${name}: invalid candidate ${n} returns false, not a trap`);
                assert.equal(e[name](n,9007199254740991,-9007199254740991,-1),0);
                assert.equal(e[name](n,5,5,1),0);
            }
            assert.equal(e.defaultExclusive(n,-9007199254740991,9007199254740991),0);
            assert.equal(e.defaultInclusive(n,-9007199254740991,9007199254740991),0);
        }
    "#;
    run_node_at_all_levels(Compiler::new(), MEMBERSHIP_SOURCE, script);
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_membership_effects_are_once_left_to_right_and_stop_on_abrupt_return() {
    let script = r#"
        const assert = require('node:assert/strict');
        const events = [];
        const e = new WebAssembly.Instance(
            new WebAssembly.Module(require('node:fs').readFileSync(0)), {
                env: { sample(tag,value) { events.push([tag,value]); return value; } }
            }).exports;
        for (const [args, expected] of [
            [[3,1,7,2],1], [[4,1,7,2],0], [[7,1,7,2],1],
            [[3,7,1,-2],1], [[3,7,1,2],0], [[5,5,5,-2],1],
            [[NaN,1,7,2],0], [[Infinity,1,7,2],0], [[-Infinity,1,7,2],0],
            [[0.5,1,7,2],0], [[9007199254740992,1,7,2],0],
        ]) {
            events.length = 0;
            const anchor = e.__alloc(8);
            assert.equal(e.main(...args),expected);
            assert.deepEqual(events,[[1,args[0]],[2,args[1]],[3,args[2]],[4,args[3]]]);
            assert.equal(e.__alloc(8),anchor+40,'one record: range evaluated exactly once');
        }
        // An invalid candidate cannot bypass evaluation or validation of the RHS.
        for (const args of [[NaN,1,7,0],[Infinity,NaN,7,2],[0.5,1,Infinity,2]]) {
            events.length = 0;
            assert.throws(() => e.main(...args),WebAssembly.RuntimeError);
            assert.deepEqual(events,[[1,args[0]],[2,args[1]],[3,args[2]],[4,args[3]]]);
        }
        for (const [name, expected] of [['skipAnd',0],['skipOr',1],['skipIf',1]]) {
            events.length = 0;
            const anchor = e.__alloc(8);
            assert.equal(e[name](),expected);
            assert.deepEqual(events,[],'skipped membership has no operand effects or trap');
            assert.equal(e.__alloc(8),anchor+8,'skipped membership allocates no record');
        }
        for (const [name, expected, log] of [
            ['abruptCandidate',1,[[1,2]]],
            ['abruptStart',0,[[1,2],[2,0]]],
            ['abruptEnd',1,[[1,2],[2,0],[3,4]]],
        ]) {
            events.length = 0;
            const anchor = e.__alloc(8);
            assert.equal(e[name](),expected);
            assert.deepEqual(events,log,'finite return prevents all later operand effects');
            assert.equal(e.__alloc(8),anchor+8,'abrupt input prevents construction');
        }
    "#;
    run_node_at_all_levels(effects_compiler(), EFFECTS_SOURCE, script);
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_membership_validates_corrupted_records_before_invalid_candidates() {
    let script = r#"
        const assert = require('node:assert/strict');
        const module = new WebAssembly.Module(require('node:fs').readFileSync(0));
        const candidates = [1,2,NaN,Infinity,-Infinity,0.5,-0.5,
            9007199254740992,-9007199254740992];
        // Adversarial writes test validation, not a supported range marshaling API.
        // The range handle remains a Wasm local: this public seam cannot forge a
        // null/misaligned/out-of-bounds handle by changing the record's fields.
        for (const [offset, value, wide] of [
            [0,9007199254740992n,true], [0,-9007199254740992n,true],
            [8,9007199254740992n,true], [8,-9007199254740992n,true],
            [16,0n,true], [16,9007199254740992n,true], [16,-9007199254740992n,true],
            [24,2,false], [24,4294967295,false], [28,1,false],
        ]) {
            for (const n of candidates) {
                let e, h, calls = 0;
                e = new WebAssembly.Instance(module, { env: { mutate() {
                    ++calls;
                    const v = new DataView(e.memory.buffer);
                    if (wide) v.setBigInt64(h+offset,value,true);
                    else v.setUint32(h+offset,value,true);
                } } }).exports;
                // No static data or other allocations: bracket the one local record.
                const anchor = e.__alloc(8);
                h = anchor+8;
                assert.throws(() => e.main(n),WebAssembly.RuntimeError,
                    `corrupt offset ${offset} with ${value}, candidate ${n}`);
                assert.equal(calls,1);
                assert.equal(e.__alloc(8),h+32,'exactly one bracketed range record');
                assert.deepEqual(Object.keys(e).sort(),['__alloc','main','memory'],
                    'no range-valued export or extra probe API');
            }
        }
        // A no-op host callback leaves the record valid: invalid candidates are false.
        let calls = 0;
        const e = new WebAssembly.Instance(module,{env:{mutate(){ ++calls; }}}).exports;
        assert.equal(e.main(1),1);
        assert.equal(e.main(2),0);
        for (const n of candidates.slice(2)) assert.equal(e.main(n),0);
        assert.equal(calls,candidates.length);
    "#;
    run_node_at_all_levels(corruption_compiler(), CORRUPTION_SOURCE, script);
}
