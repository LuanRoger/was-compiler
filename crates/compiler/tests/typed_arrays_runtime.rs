#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};
use was_compiler::{Compiler, OptimizationLevel, Type};

fn run(source: &str, script: &str) {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("effect", "env", "effect", vec![Type::Number], Type::Void)
        .unwrap();
    run_with_compiler(compiler, source, script);
}

fn run_with_compiler(mut compiler: Compiler, source: &str, script: &str) {
    let baseline = compiler.compile_source(source).unwrap();
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(source).unwrap();
        if level == OptimizationLevel::None {
            assert_eq!(bytes, baseline);
        }
        assert_eq!(compiler.compile_source(source).unwrap(), bytes);
        let program = compiler.analyze(source).unwrap();
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
        wasmparser::Validator::new().validate_all(&bytes).unwrap();
        let output = test_process::output(
            Command::new("node").arg("-e").arg(script),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn mapped_arguments_keep_written_order_defaults_aliases_and_caller_transfers() {
    run(
        r#"
        fn flags(a: bool[], b: bool[] = a) -> bool[] { if len(b) > 0 { b[0] = true; } return a; }
        fn texts(a: string[], b: string[] = a) -> string[] { b[0] = "updated"; return a; }
        fn named() -> bool[] { return flags(b: { effect(1); [] }, a: { effect(2); [false] }); }
        fn defaults() -> bool[] { return flags([false]); }
        fn string_named() -> string[] { return texts(b: { effect(3); ["other"] }, a: { effect(4); ["original"] }); }
        fn string_defaults() -> string[] { return texts(["original"]); }
        fn transfer() -> bool[] { return flags(b: { return []; [] }, a: { effect(99); [false] }); }
    "#,
        r#"
        const assert=require('node:assert/strict'), events=[];
        const e=new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)),{env:{effect:n=>events.push(n)}}).exports;
        const v=()=>new DataView(e.memory.buffer), word=p=>v().getUint32(p,true);
        const flag=h=>v().getUint8(word(h));
        const text=a=>{ const h=word(word(a)); return Buffer.from(e.memory.buffer,word(h),word(h+4)).toString('utf8'); };
        assert.equal(flag(e.named()),0); assert.deepEqual(events,[1,2]);
        assert.equal(flag(e.defaults()),1);
        assert.equal(text(e.string_named()),'original'); assert.deepEqual(events,[1,2,3,4]);
        assert.equal(text(e.string_defaults()),'updated');
        assert.equal(word(e.transfer()+4),0); assert.deepEqual(events,[1,2,3,4]);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn raw_host_array_results_validate_storage_without_scanning_elements() {
    use was_compiler::ArrayElementType;
    let mut c = Compiler::new();
    for (name, element) in [
        ("flags", ArrayElementType::Bool),
        ("texts", ArrayElementType::String),
    ] {
        c.register_host_function(name, "env", name, vec![], Type::Array(element))
            .unwrap();
    }
    run_with_compiler(
        c,
        r#"
        fn flags_len() -> number { return len(flags()); }
        fn texts_len() -> number { return len(texts()); }
        fn flag() -> bool { let a = flags(); return a[0]; }
        fn text() -> string { let a = texts(); return a[0]; }
    "#,
        r#"
        const assert=require('node:assert/strict'); let flags=0,texts=0;
        const e=new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)),{env:{flags:()=>flags,texts:()=>texts}}).exports;
        const v=()=>new DataView(e.memory.buffer); const put=(p,x)=>v().setUint32(p,x,true);
        const payload=e.__alloc(16); flags=e.__alloc(8); texts=e.__alloc(8);
        put(flags,payload+1); put(flags+4,1); v().setUint8(payload+1,2);
        put(texts,payload+4); put(texts+4,1); put(payload+4,0);
        assert.equal(e.flags_len(),1); assert.equal(e.texts_len(),1);
        assert.throws(()=>e.flag(),WebAssembly.RuntimeError); assert.throws(()=>e.text(),WebAssembly.RuntimeError);
        put(texts,payload+1); assert.throws(()=>e.texts_len(),WebAssembly.RuntimeError);
        put(flags,e.memory.buffer.byteLength-1); put(flags+4,2); assert.throws(()=>e.flags_len(),WebAssembly.RuntimeError);
        put(flags,payload); put(flags+4,0xffffffff); assert.throws(()=>e.flags_len(),WebAssembly.RuntimeError);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn construction_captures_reentry_and_traps_keep_completed_effects() {
    run(
        r#"
        fn build() -> string[] { return [{ effect(1); "left" }, { effect(2); "right" }]; }
        fn captured(n: number, a: string[]) -> string[] {
            let flag = [false];
            let mutate = () => { a[0] = "outer"; flag[0] = true; };
            mutate();
            if n > 0 { captured(0, ["inner"]); effect(3); }
            let read = () => { let leaf = () => { let empty: string[] = []; if flag[0] { a } else { empty } }; leaf() };
            return read();
        }
        fn fail(a: string[]) { a[0] = "kept"; effect(4); a[99] = "bad"; }
        fn read(a: string[]) -> string { return a[0]; }
    "#,
        r#"
        const assert=require('node:assert/strict');
        const events=[]; let reenter=false, allocationBeforeFirst=0;
        const v=()=>new DataView(e.memory.buffer), word=p=>v().getUint32(p,true);
        const text=h=>Buffer.from(e.memory.buffer,word(h),word(h+4)).toString('utf8');
        const e=new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {env:{effect:n=>{
            events.push(n);
            if(n===1) allocationBeforeFirst=e.__alloc(8);
            if(n===2) e.__alloc(70000);
            if(n===3 && !reenter) { reenter=true; const inner=e.captured(0,e.build()); assert.equal(text(e.read(inner)),'outer'); }
        }}}).exports;
        const marker=e.__alloc(8); const a=e.build();
        // Descriptor + two four-byte references are reserved before effect(1).
        assert.equal(a,marker+8); assert.equal(allocationBeforeFirst,a+16);
        assert.deepEqual(events,[1,2]); assert.equal(text(e.read(a)),'left');
        const result=e.captured(1,a); assert.equal(result,a); assert.equal(text(e.read(a)),'outer');
        assert.throws(()=>e.fail(a),WebAssembly.RuntimeError); assert.equal(text(e.read(a)),'kept');
        const later=e.build(); assert.equal(text(e.read(a)),'kept'); assert.equal(text(e.read(later)),'left');
        assert.deepEqual(events,[1,2,3,1,2,4,1,2]);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn string_handles_are_lazy_and_equality_compares_contents() {
    run(
        r#"
        fn make() -> string[] { return ["alpha", "beta", "gamma"]; }
        fn length(a: string[]) -> number { return len(a); }
        fn read(a: string[], i: number) -> string { return a[i]; }
        fn replace(a: string[], i: number, text: string) { a[i] = { effect(7); text }; }
        fn equal(a: string[], b: string[]) -> bool { return a == b; }
        fn first(a: string[]) -> string { for text in a { let outer = () => { let inner = () => text; inner() }; return outer(); } return ""; }
        fn aliases() -> string { let a = ["before", "later"]; let alias = a; let n = 0; for text in a { if n == 0 { alias[1] = "changed"; a = []; } else { return text; } n += 1; } return ""; }
    "#,
        r#"
        const assert = require('node:assert/strict');
        const events = [];
        const e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {env:{effect:n=>{ events.push(n); e.__alloc(70000); }}}).exports;
        const v = () => new DataView(e.memory.buffer);
        const word = p => v().getUint32(p,true);
        const put = (p,x) => v().setUint32(p,x,true);
        const text = h => Buffer.from(e.memory.buffer,word(h),word(h+4)).toString('utf8');
        function string(s) { const bytes=Buffer.from(s); const h=e.__alloc(8+bytes.length); put(h,h+8); put(h+4,bytes.length); new Uint8Array(e.memory.buffer,h+8,bytes.length).set(bytes); return h; }
        const a=e.make(), p=word(a); assert.equal(p%4,0); assert.equal(word(a+4),3);
        assert.deepEqual([0,1,2].map(i=>text(word(p+4*i))),['alpha','beta','gamma']);
        assert.equal(text(e.aliases()),'changed');
        const b=e.make(), q=word(b); const distinct=string('alpha'); put(q,distinct);
        assert.notEqual(word(p),word(q)); assert.equal(e.equal(a,b),1);
        put(p+8,0); assert.equal(e.length(a),3); assert.equal(text(e.first(a)),'alpha');
        assert.throws(()=>e.read(a,2),WebAssembly.RuntimeError);
        e.replace(a,2,word(q+8)); assert.equal(text(e.read(a,2)),'gamma'); assert.deepEqual(events,[7]);
        assert.throws(()=>e.replace(a,3,distinct),WebAssembly.RuntimeError); assert.deepEqual(events,[7]);
        put(q, string('different')); put(q+8,0); assert.equal(e.equal(a,b),0);
        put(q, distinct); assert.throws(()=>e.equal(a,b),WebAssembly.RuntimeError);
        put(p+8,0); assert.throws(()=>e.equal(a,a),WebAssembly.RuntimeError);
        const empty=e.__alloc(8); put(empty,q); put(empty+4,0); assert.equal(e.equal(a,empty),0);
        put(empty,q+1); assert.throws(()=>e.length(empty),WebAssembly.RuntimeError);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn boolean_layout_lazy_consumption_replacement_and_iteration() {
    run(
        r#"
        fn make() -> bool[] { return [true, false, true]; }
        fn length(a: bool[]) -> number { return len(a); }
        fn read(a: bool[], i: number) -> bool { return a[i]; }
        fn replace(a: bool[], i: number) { a[i] = { effect(7); true }; }
        fn equal(a: bool[], b: bool[]) -> bool { return a == b; }
        fn first(a: bool[]) -> bool { for flag in a { let get = () => flag; return get(); } return false; }
        fn aliases() -> bool { let a = [false, false]; let alias = a; let n = 0; for flag in a { if n == 0 { alias[1] = true; a = []; } else { return flag; } n += 1; } return false; }
    "#,
        r#"
        const assert = require('node:assert/strict');
        const events = [];
        const e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {env:{effect:n=>events.push(n)}}).exports;
        const view = () => new DataView(e.memory.buffer);
        const a = e.make(); const p = view().getUint32(a,true);
        assert.equal(view().getUint32(a+4,true),3);
        assert.deepEqual([...new Uint8Array(e.memory.buffer,p,3)], [1,0,1]);
        assert.equal(e.aliases(),1);
        view().setUint8(p+2,2);
        assert.equal(e.length(a),3); assert.equal(e.first(a),1);
        assert.throws(()=>e.read(a,2), WebAssembly.RuntimeError);
        e.replace(a,2); assert.deepEqual(events,[7]); assert.equal(e.read(a,2),1);
        assert.throws(()=>e.replace(a,99), WebAssembly.RuntimeError); assert.deepEqual(events,[7]);
        const b = e.make(); const q = view().getUint32(b,true);
        view().setUint8(p,0); view().setUint8(q+2,2);
        assert.equal(e.equal(a,b),0);
        view().setUint8(p,1);
        assert.throws(()=>e.equal(a,b), WebAssembly.RuntimeError);
        view().setUint8(p,2);
        assert.throws(()=>e.equal(a,a), WebAssembly.RuntimeError);
        const short = e.__alloc(8); view().setUint32(short,p,true); view().setUint32(short+4,0,true);
        assert.equal(e.equal(a,short),0);
        view().setUint32(short,0,true); assert.throws(()=>e.equal(a,short),WebAssembly.RuntimeError);
    "#,
    );
}
