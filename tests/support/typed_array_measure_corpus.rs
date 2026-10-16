//! M4 matched-width controls, separate from the immutable M0 corpus.

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
    for (element, on, off) in [
        ("number", "1", "0"),
        ("bool", "true", "false"),
        ("string", "\"éabcdefg\"", "\"éabcdefx\""),
    ] {
        for length in [0, 1, 2, 3, 7, 8, 9] {
            let elements = vec![on; length].join(",");
            cases.push(Case {
                name: format!("{element}_construct_{length}"),
                source: repeated(
                    "",
                    &format!("let a: {element}[] = [{elements}]; total += len(a);"),
                    128,
                ),
            });
        }
        let full = format!("[{}]", [on; 8].join(","));
        cases.push(Case {
            name: format!("{element}_page_8"),
            source: repeated("", &format!("let a = {full}; total += len(a);"), 8192),
        });
        cases.push(Case {
            name: format!("{element}_alias_replace"),
            source: repeated(
                &format!("let a = [{on},{off},{on}]; let alias = a;"),
                &format!("alias[1] = a[0]; if a[1] == {on} {{ total += 1; }} total += len(alias);"),
                128,
            ),
        });
        let four = format!("let a = [{on},{off},{on},{off}];");
        cases.push(Case {
            name: format!("{element}_iteration_live"),
            source: repeated(
                &format!("{four} let alias = a;"),
                &format!("alias[3] = {off}; for x in a {{ if x == {on} {{ total += 1; }} alias[3] = {on}; }}"),
                128,
            ),
        });
        for (exit, rhs) in [
            ("length", format!("[{on}]")),
            ("early", format!("[{off},{}]", [on; 7].join(","))),
            ("late", format!("[{}, {off}]", [on; 7].join(","))),
            ("equal", full.clone()),
        ] {
            cases.push(Case {
                name: format!("{element}_equality_{exit}"),
                source: repeated(
                    &format!("let a = {full}; let b = {rhs};"),
                    "if a == b { total += 1; }",
                    128,
                ),
            });
        }
        cases.push(Case {
            name: format!("{element}_capture_shared"),
            source: repeated(
                &format!(
                    "let value = {off}; let a = [{off}]; let alias = a; \
                    let update = () => {{ value = {on}; a[0] = value; value }};"
                ),
                &format!("update(); if alias[0] == {on} {{ total += 1; }}"),
                128,
            ),
        });
        cases.push(Case {
            name: format!("{element}_capture_iteration"),
            source: repeated(
                &four,
                &format!(
                    "for x in a {{ let read = () => x; if read() == {on} {{ total += 1; }} }}"
                ),
                128,
            ),
        });
    }
    cases
}
