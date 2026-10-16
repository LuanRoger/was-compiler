//! Bounded M0 source workloads; deliberately independent of production layout constants.

pub struct Case {
    pub name: String,
    pub source: String,
}

fn repeated(setup: &str, body: &str, count: usize) -> String {
    format!(
        "fn main() -> number {{ {setup} let total = 0; let i = 0; \
         while i < {count} {{ {body} i += 1; }} return total; }}"
    )
}

pub fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for length in [0, 1, 7, 8, 9] {
        let elements = vec!["1"; length].join(",");
        cases.push(Case {
            name: format!("construct_{length}"),
            source: repeated("", &format!("let a = [{elements}]; total += len(a);"), 128),
        });
    }
    cases.push(Case {
        name: "page_crossing".into(),
        source: repeated("", "let a = [1]; total += a[0];", 8192),
    });
    cases.push(Case {
        name: "alias_index_replace".into(),
        source: repeated(
            "let a = [1,2,3]; let alias = a;",
            "alias[1] = a[0] + 8; total += a[1] + len(alias);",
            128,
        ),
    });
    cases.push(Case {
        name: "iteration_live_alias".into(),
        source: repeated(
            "let a = [1,2,3,4]; let alias = a;",
            "alias[3] = 4; for x in a { total += x; alias[3] = 8; }",
            128,
        ),
    });
    for (name, right) in [
        ("equality_length", "[1]"),
        ("equality_early", "[9,2,3,4,5,6,7,8]"),
        ("equality_late", "[1,2,3,4,5,6,7,9]"),
        ("equality_equal", "[1,2,3,4,5,6,7,8]"),
    ] {
        cases.push(Case {
            name: name.into(),
            source: repeated(
                &format!("let a = [1,2,3,4,5,6,7,8]; let b = {right};"),
                "if a == b { total += 1; }",
                128,
            ),
        });
    }
    // UTF-8 payloads straddle raw allocator alignment boundaries (7/8/9 bytes).
    for (name, left, right) in [
        ("string_7_equal", "éabcde", "éabcde"),
        ("string_8_early", "éabcdef", "xabcdefg"),
        ("string_9_late", "éabcdefg", "éabcdefx"),
    ] {
        cases.push(Case {
            name: name.into(),
            source: repeated(
                &format!("let a = \"{left}\"; let b = \"{right}\";"),
                "if a == b { total += 1; }",
                128,
            ),
        });
    }
    cases.push(Case {
        name: "capture_shared".into(),
        source: repeated(
            "let value = 0; let a = [0]; let alias = a; \
                         let update = () => { value += 1; a[0] = value; value };",
            "update(); total += alias[0];",
            128,
        ),
    });
    cases.push(Case {
        name: "capture_iteration".into(),
        source: repeated(
            "let a = [1,2,3,4];",
            "for x in a { let read = () => x; total += read(); }",
            128,
        ),
    });
    cases
}
