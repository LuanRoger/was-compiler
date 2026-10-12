#[path = "support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{
    CodegenError, Compiler, ExportOptions, OptimizationLevel, Type,
    typed_ast::{LoopId, TypedExpr, TypedExprKind, TypedStatementKind},
};
use wasmparser::{BlockType, ExternalKind, Operator, Parser, Payload, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const PROGRESSION_SOURCE: &str = r#"
    fn exclusive(a: number, b: number, s: number) -> number {
        let count = 0;
        for i in a..b by s { emit(i); count += 1; }
        return count;
    }
    fn inclusive(a: number, b: number, s: number) -> number {
        let count = 0;
        for i in a..=b by s { emit(i); count += 1; }
        return count;
    }
    fn defaultExclusive(a: number, b: number) -> number {
        let count = 0;
        for i in a..b { emit(i); count += 1; }
        return count;
    }
    fn defaultInclusive(a: number, b: number) -> number {
        let count = 0;
        for i in a..=b { emit(i); count += 1; }
        return count;
    }
    fn containsExclusive(n: number, a: number, b: number, s: number) -> bool {
        return n in a..b by s;
    }
    fn containsInclusive(n: number, a: number, b: number, s: number) -> bool {
        return n in a..=b by s;
    }
"#;

const EFFECTS_SOURCE: &str = r#"
    fn make(a: number, b: number, s: number) -> range {
        sample(5, 0);
        return a..=b by s;
    }
    fn identity(r: range) -> range { sample(6, 0); return r; }
    fn main(a: number, b: number, s: number) -> number {
        let count = 0;
        for i in {
            sample(1, 0);
            identity(make(sample(2, a), sample(3, b), sample(4, s)))
        } { emit(i); count += 1; }
        return count;
    }
    fn direct(a: number, b: number, s: number) -> number {
        let count = 0;
        for i in sample(1, a)..=sample(2, b) by sample(3, s) {
            emit(i); count += 1;
        }
        return count;
    }
    fn reuse(flag: bool) -> number {
        let r = 1..=5 by 2;
        let copy = r;
        let count = 0;
        for i in if flag { { copy } } else { loop { break r; } } {
            emit(i); count += 1;
        }
        return count;
    }
    fn skip() -> bool {
        return false && { for i in sample(1, 0)..sample(2, 3) by sample(3, 0) {
            emit(i);
        } true };
    }
"#;

const CONTROL_SOURCE: &str = r#"
    fn transfers() -> number {
        for i in 0..6 {
            emit(i);
            false && { continue; true };
            true || { break; false };
            let n = 100 + if i == 1 { { continue; 0 } } else { 10 };
            emit(n);
            true && { if i == 3 { { break; }; } true };
            emit(200 + i);
        }
        return 9;
    }
    fn logicalContinue() -> number {
        for i in 0..5 {
            emit(i);
            true && { if i < 2 { { continue; }; } true };
            emit(100 + i);
            if i == 3 { break; }
        }
        return 11;
    }
    fn returnBody() -> number {
        for i in 0..5 {
            emit(i);
            let n = 50 + if i == 2 { { true && { return i; true }; 0 } } else { 1 };
            emit(n);
        }
        return -1;
    }
    fn nested() -> number {
        let total = 0;
        for i in 1..=2 {
            let w = 0;
            while w < 2 {
                w += 1;
                if w == 1 { continue; }
                for j in 0..=2 {
                    if j == 1 { continue; }
                    let n = loop {
                        let k = 0;
                        for k in 0..=2 {
                            if k == 0 { continue; }
                            emit(100 * i + 10 * j + k);
                            break;
                        }
                        break 10 * i + j;
                    };
                    total += n;
                    if j == 2 { break; }
                }
            }
        }
        return total;
    }
    fn shadow() -> number {
        let i = 99;
        let total = 0;
        for i in 0..3 {
            emit(i);
            { let i = i; i += 10; emit(i); };
            emit(i);
            let i = i;
            i = i + 20;
            i %= 7;
            total += i;
        }
        emit(i);
        i += 1;
        return total + i;
    }
    fn hidden() -> void {
        return;
        for i in 0..0 { let a = [i]; a[0] %= 2; continue; emit(999); }
    }
"#;

const ITERABLE_SOURCE: &str = r#"
    fn outerBreak() -> number {
        let n = 0;
        loop {
            n += 1;
            for i in { sample(1, n); break; 0..1 } { emit(999); }
            emit(998);
        }
        return n;
    }
    fn outerContinue() -> number {
        let n = 0;
        while n < 3 {
            n += 1;
            for i in { sample(1, n); if n < 3 { continue; } 0..=1 } { emit(i); }
            emit(100 + n);
        }
        return n;
    }
    fn valueBreak() -> number {
        return 10 + loop {
            for i in { sample(1, 5); break 7; 0..1 } { emit(999); }
            break 99;
        };
    }
    fn returnIterable() -> number {
        for i in sample(1, 0)..{ sample(2, 3); return 17; 3 } by sample(3, 1) {
            emit(999);
        }
        return 99;
    }
    fn nestedIterable() -> number {
        for i in 0..3 {
            for j in {
                sample(1, i);
                if i == 1 { continue; }
                if i == 2 { break; }
                0..=i
            } { emit(10 * i + j); }
            emit(100 + i);
        }
        return 23;
    }
    fn condition() -> number {
        while { for i in { sample(1, 0); break; 0..1 } { emit(999); } true } {
            emit(998);
        }
        return 21;
    }
    fn names() -> number {
        let i = 1;
        for i in { i += 1; i..=4 } { emit(i); }
        return i;
    }
    fn onlyReturn() -> number {
        for i in { sample(1, 8); return 8; } { emit(999); }
    }
    fn divergent() -> number {
        for i in loop {} { emit(999); }
    }
"#;

const CORRUPTION_SOURCE: &str = r#"
    fn main(a: number, b: number, s: number) -> number {
        let r = a..=b by s;
        mutate();
        for i in r { emit(i); }
        return 77;
    }
    fn cached() -> number {
        let r = 1..=5 by 2;
        for i in r { emit(i); mutate(); }
        return 77;
    }
"#;

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    for (name, params, result) in [
        ("sample", vec![Type::Number, Type::Number], Type::Number),
        ("emit", vec![Type::Number], Type::Void),
        ("mutate", vec![], Type::Void),
    ] {
        compiler
            .register_host_function(name, "env", name, params, result)
            .unwrap();
    }
    compiler
        .register_host_function("unused", "other", "unused", vec![], Type::Bool)
        .unwrap();
    compiler
}

fn effects_compiler() -> Compiler {
    let mut compiler = compiler();
    compiler.set_exports(ExportOptions::Only(
        ["main", "direct", "reuse", "skip"]
            .into_iter()
            .map(String::from)
            .collect(),
    ));
    compiler
}

fn compile(compiler: &Compiler, source: &str) -> Vec<u8> {
    let bytes = compiler
        .compile_source(source)
        .expect("compile range for loops");
    Validator::new()
        .validate_all(&bytes)
        .expect("valid for-loop Wasm");
    bytes
}

fn non_export_sections(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    Parser::new(0)
        .parse_all(bytes)
        .filter_map(|payload| {
            let (id, range) = payload.unwrap().as_section()?;
            (id != 7).then(|| {
                let start = usize::try_from(range.start).expect("section start fits usize");
                let end = usize::try_from(range.end).expect("section end fits usize");
                (id, bytes[start..end].to_vec())
            })
        })
        .collect()
}

fn run_node_at_all_levels(mut compiler: Compiler, source: &str, script: &str) {
    let baseline = compile(&compiler, source);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compile(&compiler, source);
        assert_eq!(bytes, baseline, "{level:?}");
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
fn public_fixtures_validate_and_are_deterministic_at_every_optimization_level() {
    for (mut compiler, source) in [
        (compiler(), PROGRESSION_SOURCE),
        (effects_compiler(), EFFECTS_SOURCE),
        (compiler(), CONTROL_SOURCE),
        (compiler(), ITERABLE_SOURCE),
        (compiler(), CORRUPTION_SOURCE),
    ] {
        let program = compiler.analyze(source).unwrap();
        let original = program.clone();
        let baseline = compile(&compiler, source);
        for level in LEVELS {
            compiler.set_optimization_level(level);
            for _ in 0..2 {
                assert_eq!(compiler.analyze(source).unwrap(), original, "{level:?}");
                assert_eq!(compile(&compiler, source), baseline, "{level:?}");
                assert_eq!(compiler.compile(&program).unwrap(), baseline, "{level:?}");
                assert_eq!(program, original);
            }
        }
    }
}

#[test]
fn owned_import_snapshots_and_export_selection_preserve_bodies_and_indices() {
    let mut compiler = compiler();
    let program = compiler.analyze(PROGRESSION_SOURCE).unwrap();
    let original = program.clone();
    let baseline = compiler.compile(&program).unwrap();
    compiler
        .register_host_function("later", "later", "added", vec![], Type::Void)
        .unwrap();
    assert_ne!(compile(&compiler, PROGRESSION_SOURCE), baseline);

    for (selection, names) in [
        (
            ExportOptions::All,
            vec![
                "exclusive",
                "inclusive",
                "defaultExclusive",
                "defaultInclusive",
                "containsExclusive",
                "containsInclusive",
            ],
        ),
        (ExportOptions::None, vec![]),
        (
            ExportOptions::Only(vec!["inclusive".into(), "exclusive".into()]),
            vec!["exclusive", "inclusive"],
        ),
    ] {
        compiler.set_exports(selection);
        let expected = compiler.compile(&program).unwrap();
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile(&program).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            assert_eq!(bytes, expected);
            assert_eq!(non_export_sections(&bytes), non_export_sections(&baseline));
            let mut imports = Vec::new();
            let mut exports = Vec::new();
            for payload in Parser::new(0).parse_all(&bytes) {
                match payload.unwrap() {
                    Payload::ImportSection(section) => {
                        for import in section.into_imports() {
                            let import = import.unwrap();
                            imports.push((import.module.to_owned(), import.name.to_owned()));
                        }
                    }
                    Payload::ExportSection(section) => {
                        for export in section {
                            let export = export.unwrap();
                            exports.push((export.name.to_owned(), export.kind, export.index));
                        }
                    }
                    _ => {}
                }
            }
            assert_eq!(
                imports,
                [
                    ("env".into(), "sample".into()),
                    ("env".into(), "emit".into()),
                    ("env".into(), "mutate".into()),
                    ("other".into(), "unused".into())
                ]
            );
            assert_eq!(
                exports
                    .iter()
                    .filter(
                        |(_, kind, index)| *kind == ExternalKind::Func && (4..10).contains(index)
                    )
                    .map(|(name, _, _)| name.as_str())
                    .collect::<Vec<_>>(),
                names
            );
            for (name, kind, index) in &exports {
                if let Some(position) = program.functions.iter().position(|f| f.name == *name) {
                    assert_eq!(*kind, ExternalKind::Func);
                    assert_eq!(*index, u32::try_from(position + 4).unwrap());
                }
            }
            assert!(exports.contains(&("memory".into(), ExternalKind::Memory, 0)));
            assert!(exports.contains(&("__alloc".into(), ExternalKind::Func, 10)));
            assert_eq!(program, original);
        }
    }
}

#[test]
fn decoded_for_uses_integer_induction_empty_regions_and_a_continue_latch() {
    let source = "fn main(flag: bool) -> void {
        for i in 0..3 { if flag { continue; } break; }
    }";
    let bytes = compile(&Compiler::new(), source);
    let body = Parser::new(0)
        .parse_all(&bytes)
        .find_map(|payload| match payload.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .unwrap();
    let locals = body
        .get_locals_reader()
        .unwrap()
        .into_iter()
        .flat_map(|local| {
            let (count, ty) = local.unwrap();
            std::iter::repeat_n(ty, usize::try_from(count).unwrap())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        locals.iter().filter(|ty| **ty == ValType::F64).count(),
        1,
        "the iteration binding is a language number"
    );
    assert_eq!(
        locals.iter().filter(|ty| **ty == ValType::I64).count(),
        3,
        "current, end and step are exact cached integers"
    );
    assert!(
        locals.iter().filter(|ty| **ty == ValType::I32).count() >= 2,
        "private handle and cached inclusivity"
    );

    let mut branches = Vec::new();
    let mut regions = Vec::new();
    let mut integer_adds = 0;
    let mut yields = 0;
    for op in body.get_operators_reader().unwrap() {
        match op.unwrap() {
            Operator::Block { blockty } | Operator::Loop { blockty } => regions.push(blockty),
            Operator::Br { relative_depth } => branches.push(relative_depth),
            Operator::I64Add => integer_adds += 1,
            Operator::F64ConvertI64S => yields += 1,
            Operator::F64Add => panic!("for advancement must not round in f64"),
            _ => {}
        }
    }
    assert_eq!(
        regions.len(),
        3,
        "exit block, backedge loop, body latch block"
    );
    assert!(regions.iter().all(|ty| *ty == BlockType::Empty));
    assert_eq!(
        branches,
        [1, 2, 0],
        "continue exits the body block; break exits the for; backedge restarts retest"
    );
    assert_eq!(integer_adds, 1);
    assert_eq!(yields, 1);
}

#[test]
fn nested_and_sibling_fors_reserve_distinct_scratch_without_new_helpers() {
    let source = "fn main() -> void {
        for i in 0..1 { for j in 0..1 {} }
        for k in 0..1 {}
    }";
    let mut compiler = Compiler::new();
    let baseline = compile(&compiler, source);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compile(&compiler, source);
        assert_eq!(bytes, baseline);
        let mut function_count = 0;
        let mut bodies = Vec::new();
        for payload in Parser::new(0).parse_all(&bytes) {
            match payload.unwrap() {
                Payload::FunctionSection(section) => function_count = section.count(),
                Payload::CodeSectionEntry(body) => bodies.push(body),
                Payload::ImportSection(section) => {
                    assert_eq!(section.count(), 0, "no implicit for imports");
                }
                _ => {}
            }
        }
        assert_eq!(
            function_count, 10,
            "one user, seven memory, construct and validate; no iteration helper"
        );
        assert_eq!(bodies.len(), 10);
        let locals = bodies[0]
            .get_locals_reader()
            .unwrap()
            .into_iter()
            .flat_map(|local| {
                let (count, ty) = local.unwrap();
                std::iter::repeat_n(ty, usize::try_from(count).unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(locals.iter().filter(|ty| **ty == ValType::F64).count(), 3);
        assert_eq!(
            locals.iter().filter(|ty| **ty == ValType::I64).count(),
            9,
            "three exact induction slots per for, without sibling reuse"
        );
        assert!(locals.iter().filter(|ty| **ty == ValType::I32).count() >= 6);
    }
}

#[test]
fn hidden_unreachable_for_bodies_still_reject_forged_targets_and_value_breaks() {
    for prefix in ["", "return;"] {
        let source = format!(
            "fn hidden() -> void {{ {prefix}
            for i in 0..0 {{ continue; }} }} fn main() -> void {{}}"
        );
        let mut compiler = Compiler::new();
        let original = compiler.analyze(&source).unwrap();
        for valued in [false, true] {
            let mut program = original.clone();
            let TypedStatementKind::For { id, body, .. } =
                &mut program.functions[0].body.last_mut().unwrap().kind
            else {
                panic!("expected for");
            };
            let span = body[0].span;
            let target = LoopId(usize::MAX);
            let expected = if valued {
                body[0].kind = TypedStatementKind::BreakValue {
                    target: *id,
                    value: TypedExpr {
                        kind: TypedExprKind::Number("1".into()),
                        ty: Type::Number.into(),
                    },
                };
                CodegenError::BreakValueNotAllowed {
                    function: "hidden".into(),
                    span,
                }
            } else {
                body[0].kind = TypedStatementKind::Continue { target };
                CodegenError::InvalidLoopTarget {
                    function: "hidden".into(),
                    target,
                    span,
                }
            };
            let snapshot = program.clone();
            for exports in [
                ExportOptions::None,
                ExportOptions::Only(vec!["main".into()]),
            ] {
                compiler.set_exports(exports);
                for level in LEVELS {
                    compiler.set_optimization_level(level);
                    assert_eq!(compiler.compile(&program).unwrap_err(), expected);
                    assert_eq!(program, snapshot);
                }
            }
        }
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_exact_progressions_match_bounded_bigint_reference_and_membership() {
    let script = r#"
        const assert = require('node:assert/strict');
        let yielded = [];
        const e = new WebAssembly.Instance(
            new WebAssembly.Module(require('node:fs').readFileSync(0)), {
                env: { sample(t,v) { return v; }, mutate() {}, emit(n) {
                    assert.ok(yielded.length < 64, 'guest iteration cap');
                    yielded.push(n);
                } }, other: { unused() { return 0; } }
            }).exports;
        // BigInt arithmetic is independent of guest i64/f64 lowering. Refuse long
        // references before invoking the guest; host emit also bounds buggy yields.
        function reference(a,b,s,inclusive) {
            a = BigInt(a); b = BigInt(b); s = BigInt(s);
            const values = [];
            for (let n = a; s > 0n ? (inclusive ? n <= b : n < b)
                                   : (inclusive ? n >= b : n > b); n += s) {
                assert.ok(values.length < 64, 'reference iteration cap');
                values.push(Number(n));
            }
            return values;
        }
        function check(name,a,b,s,inclusive,defaultStep = false,literal) {
            const expected = reference(a,b,s,inclusive);
            if (literal) assert.deepEqual(expected,literal);
            yielded = [];
            assert.equal(defaultStep ? e[name](a,b) : e[name](a,b,s),expected.length);
            assert.deepEqual(yielded,expected,`${name}(${a},${b},${s})`);
            for (const n of yielded) {
                assert.equal(e[inclusive ? 'containsInclusive' : 'containsExclusive'](n,a,b,s),1);
                if (n === 0) assert.ok(Object.is(n,+0) && !Object.is(n,-0),'canonical yielded +0');
            }
            // Checking a few nonmembers also catches extra/skipped progression values.
            for (const n of [a,b,0,a+1,a-1,b+1,b-1]) {
                if (!Number.isSafeInteger(n)) continue;
                assert.equal(e[inclusive ? 'containsInclusive' : 'containsExclusive'](n,a,b,s),
                    expected.includes(n) ? 1 : 0);
            }
        }
        const M = 9007199254740991;
        for (const [a,b,s] of [
            [0,3,1], [5,5,1], [5,5,-1], [1,7,2], [1,6,2],
            [7,1,-2], [7,2,-2], [1,7,-2], [7,1,2],
            [-0,0,1], [0,-0,-1], [-2,2,1], [2,-2,-1],
            [0,3,M], [0,-3,-M], [M-2,M,1], [-M+2,-M,-1],
            [M-4,M,3], [-M+4,-M,-3], [M-1,M,M], [-M+1,-M,-M],
            [M,M,M], [-M,-M,-M], [-M,M,M], [M,-M,-M],
            [-M,M,M-1], [M,-M,-M+1],
        ]) {
            check('exclusive',a,b,s,false);
            check('inclusive',a,b,s,true);
        }
        check('inclusive',-M,M,M,true,false,[-M,0,M]);
        check('inclusive',M,-M,-M,true,false,[M,0,-M]);
        check('inclusive',-M,M,M-1,true,false,[-M,-1,M-2]);
        check('inclusive',M,-M,-M+1,true,false,[M,1,-M+2]);
        for (const [a,b] of [[0,3],[5,5],[3,0],[-0,0],[-M,-M+3],[M-3,M]]) {
            check('defaultExclusive',a,b,1,false,true);
            check('defaultInclusive',a,b,1,true,true);
        }
        // Small exhaustive cases keep the oracle and guest bounded, including
        // all sign/direction/inclusivity combinations, not just selected examples.
        for (let a = -3; a <= 3; ++a) for (let b = -3; b <= 3; ++b) {
            for (const s of [-3,-2,-1,1,2,3]) {
                check('exclusive',a,b,s,false);
                check('inclusive',a,b,s,true);
            }
        }
    "#;
    run_node_at_all_levels(compiler(), PROGRESSION_SOURCE, script);
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_iterable_and_endpoints_evaluate_once_left_to_right_before_validation() {
    let script = r#"
        const assert = require('node:assert/strict');
        let events = [], yielded = [];
        const e = new WebAssembly.Instance(
            new WebAssembly.Module(require('node:fs').readFileSync(0)), {
                env: { sample(t,v) { events.push([t,v]); return v; }, mutate() {}, emit(n) {
                    assert.ok(yielded.length < 16,'guest iteration cap');
                    yielded.push(n); events.push(['emit',n]);
                } }, other: { unused() { return 0; } }
            }).exports;
        for (const [args,expected] of [
            [[1,5,2],[1,3,5]], [[5,1,-2],[5,3,1]], [[5,1,2],[]], [[5,5,-2],[5]],
        ]) {
            events = []; yielded = [];
            const anchor = e.__alloc(8);
            assert.equal(e.main(...args),expected.length);
            assert.deepEqual(events,[[1,0],[2,args[0]],[3,args[1]],[4,args[2]],[5,0],[6,0],
                            ...expected.map(n => ['emit',n])]);
            assert.deepEqual(yielded,expected);
            assert.equal(e.__alloc(8),anchor+40,'iterable constructs just one 32-byte record');
        }
        const invalid = [NaN,Infinity,-Infinity,0.5,-0.5,Number.MIN_VALUE,
            -Number.MIN_VALUE,9007199254740992,-9007199254740992,Number.MAX_VALUE];
        for (let field = 0; field < 3; ++field) {
            for (const value of [...invalid,...(field === 2 ? [0,-0] : [])]) {
                const args = [5,1,1]; // Empty direction still must validate.
                args[field] = value;
                events = []; yielded = [];
                assert.throws(() => e.direct(...args),WebAssembly.RuntimeError);
                assert.deepEqual(events,[[1,args[0]],[2,args[1]],[3,args[2]]],
                    'all numeric operands evaluate before construction traps');
                assert.deepEqual(yielded,[]);
            }
        }
        for (const flag of [0,1]) {
            events = []; yielded = [];
            const anchor = e.__alloc(8);
            assert.equal(e.reuse(flag),3);
            assert.deepEqual(yielded,[1,3,5]);
            assert.deepEqual(events,[['emit',1],['emit',3],['emit',5]]);
            assert.equal(e.__alloc(8),anchor+40,'aliases/block/if/value-loop add no records');
        }
        events = []; yielded = [];
        const anchor = e.__alloc(8);
        assert.equal(e.skip(),0);
        assert.deepEqual(events,[]); assert.deepEqual(yielded,[]);
        assert.equal(e.__alloc(8),anchor+8,'short-circuit skips invalid iterable entirely');
    "#;
    run_node_at_all_levels(effects_compiler(), EFFECTS_SOURCE, script);
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_body_transfers_advance_through_nested_regions_and_restore_shadow_scopes() {
    let script = r#"
        const assert = require('node:assert/strict');
        let events = [];
        const e = new WebAssembly.Instance(
            new WebAssembly.Module(require('node:fs').readFileSync(0)), {
                env: { sample(t,v) { return v; }, mutate() {}, emit(n) {
                    assert.ok(events.length < 64,'guest iteration cap'); events.push(n);
                } }, other: { unused() { return 0; } }
            }).exports;
        function check(name,result,expected) {
            events = [];
            assert.equal(e[name](),result);
            assert.deepEqual(events,expected,name);
        }
        check('transfers',9,[0,110,200,1,2,110,202,3,110]);
        check('logicalContinue',11,[0,1,2,102,3,103]);
        check('returnBody',2,[0,51,1,51,2]);
        check('nested',64,[101,121,201,221]);
        check('shadow',107,[0,10,0,1,11,1,2,12,2,99]);
        check('hidden',undefined,[]);
    "#;
    run_node_at_all_levels(compiler(), CONTROL_SOURCE, script);
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_iterable_transfers_use_enclosing_targets_and_return_before_body() {
    let script = r#"
        const assert = require('node:assert/strict');
        let events = [];
        const e = new WebAssembly.Instance(
            new WebAssembly.Module(require('node:fs').readFileSync(0)), {
                env: { sample(t,v) {
                    assert.ok(events.length < 32,'iterable effect cap');
                    events.push(['sample',t,v]); return v;
                }, mutate() {}, emit(n) {
                    assert.ok(events.length < 32,'guest iteration cap'); events.push(['emit',n]);
                } }, other: { unused() { return 0; } }
            }).exports;
        for (const [name,result,expected] of [
            ['outerBreak',1,[['sample',1,1]]],
            ['outerContinue',3,[['sample',1,1],['sample',1,2],['sample',1,3],
                ['emit',0],['emit',1],['emit',103]]],
            ['valueBreak',17,[['sample',1,5]]],
            ['returnIterable',17,[['sample',1,0],['sample',2,3]]],
            ['nestedIterable',23,[['sample',1,0],['emit',0],['emit',100],
                ['sample',1,1],['sample',1,2]]],
            ['condition',21,[['sample',1,0]]],
            ['names',2,[['emit',2],['emit',3],['emit',4]]],
                        ['onlyReturn',8,[['sample',1,8]]],
        ]) {
            events = [];
            const anchor = e.__alloc(8);
            assert.equal(e[name](),result);
            assert.deepEqual(events,expected,name);
            if (['outerBreak','valueBreak','returnIterable','condition','onlyReturn'].includes(name)) {
                assert.equal(e.__alloc(8),anchor+8,'abrupt iterable never constructs a record');
            }
        }
    "#;
    run_node_at_all_levels(compiler(), ITERABLE_SOURCE, script);
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_consumed_records_validate_before_yield_and_fields_are_cached_before_body() {
    let script = r#"
        const assert = require('node:assert/strict');
        const wasm = new WebAssembly.Module(require('node:fs').readFileSync(0));
        // Adversarial private-record writes are NOT supported range marshaling or
        // an immutability guarantee against malicious hosts. They isolate entry
        // validation and the ticket's cached-induction contract through public API.
        for (const [offset,value,wide] of [
            [0,9007199254740992n,true], [0,-9007199254740992n,true],
            [8,9007199254740992n,true], [8,-9007199254740992n,true],
            [16,0n,true], [16,9007199254740992n,true], [16,-9007199254740992n,true],
            [24,2,false], [24,4294967295,false], [28,1,false],
        ]) for (const args of [[1,5,2],[5,1,2],[5,5,-2]]) {
            let e, h, calls = 0, yielded = [];
            e = new WebAssembly.Instance(wasm, {
                env: { sample(t,v) { return v; }, emit(n) {
                    assert.ok(yielded.length < 16,'guest iteration cap'); yielded.push(n);
                }, mutate() {
                    ++calls;
                    const v = new DataView(e.memory.buffer);
                    if (wide) v.setBigInt64(h+offset,value,true);
                    else v.setUint32(h+offset,value,true);
                } }, other: { unused() { return 0; } }
            }).exports;
            // Bracket the only allocation; no static data or public range export.
            const anchor = e.__alloc(8); h = anchor+8;
            assert.throws(() => e.main(...args),WebAssembly.RuntimeError,
                `offset ${offset}, value ${value}, args ${args}`);
            assert.equal(calls,1);
            assert.deepEqual(yielded,[],'validate even empty ranges before any yield');
            assert.equal(e.__alloc(8),h+32);
        }
        for (const corrupt of [false,true]) {
            let e, h, calls = 0, yielded = [];
            e = new WebAssembly.Instance(wasm, {
                env: { sample(t,v) { return v; }, emit(n) {
                    assert.ok(yielded.length < 16,'guest iteration cap'); yielded.push(n);
                }, mutate() {
                    ++calls;
                    const v = new DataView(e.memory.buffer);
                    v.setBigInt64(h,99n,true);
                    v.setBigInt64(h+8,99n,true);
                    v.setBigInt64(h+16,corrupt ? 0n : -1n,true);
                    v.setUint32(h+24,corrupt ? 2 : 0,true);
                    v.setUint32(h+28,corrupt ? 1 : 0,true);
                } }, other: { unused() { return 0; } }
            }).exports;
            h = e.__alloc(8)+8;
            assert.equal(e.cached(),77,'body mutation cannot change cached progress or retrap');
            assert.deepEqual(yielded,[1,3,5]);
            assert.equal(calls,3);
            assert.equal(e.__alloc(8),h+32);
        }
        let yielded = [];
        const e = new WebAssembly.Instance(wasm, {
            env: { sample(t,v) { return v; }, mutate() {}, emit(n) {
                assert.ok(yielded.length < 16,'guest iteration cap'); yielded.push(n);
            } }, other: { unused() { return 0; } }
        }).exports;
        for (const [args,expected] of [[[1,5,2],[1,3,5]],[[5,1,2],[]],[[5,5,-2],[5]]]) {
            yielded = []; assert.equal(e.main(...args),77); assert.deepEqual(yielded,expected);
        }
    "#;
    run_node_at_all_levels(compiler(), CORRUPTION_SOURCE, script);
}
