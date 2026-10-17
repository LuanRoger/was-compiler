use was_compiler::{Compiler, CompilerOptions, OptimizationLevel, Type};

pub struct Case {
    pub name: String,
    pub source: String,
}

pub fn compiler(optimization_level: OptimizationLevel) -> Compiler {
    let mut compiler = Compiler::with_options(CompilerOptions { optimization_level });
    compiler
        .register_host_function("mark", "env", "mark", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
}

pub fn measurement_level() -> OptimizationLevel {
    match std::env::var("INTEGER_BASELINE_LEVEL")
        .as_deref()
        .unwrap_or("None")
    {
        "None" => OptimizationLevel::None,
        "Basic" => OptimizationLevel::Basic,
        "Standard" => OptimizationLevel::Standard,
        "Aggressive" => OptimizationLevel::Aggressive,
        other => panic!("unknown INTEGER_BASELINE_LEVEL: {other}"),
    }
}

impl std::fmt::Display for Case {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

pub fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for size in [1, 32] {
        for (family, statement) in [
            ("literal", "total += (120 + 30) - 7;"),
            (
                "locals",
                "{ let a = 120; let b = a + 30; let c = b - 7; total += c; };",
            ),
            (
                "compare",
                "{ let a = 120 + 30; if a > 149 { total += 1; } if a == 150 { total += 2; } if a <= 149 { total += 100; } else { total += 0; } };",
            ),
            (
                "negate",
                "{ let a = 120 + 30; let b = -a; total += b + 7; };",
            ),
        ] {
            let mut source = "fn main(x: number) -> number { let total = 0;\n".to_owned();
            for _ in 0..size {
                source.push_str(statement);
                source.push('\n');
            }
            source.push_str("return total; }");
            cases.push(Case {
                name: format!("{family}_{size}"),
                source,
            });
        }
    }
    for (name, body) in [
        (
            "loop_local",
            "let total = 0; for i in 0..32 { let a = 120; let b = a + 23; total += b; } return total;",
        ),
        ("fraction", "return 1.5 + 2.25;"),
        ("negative_zero", "return -0;"),
        ("negate_zero", "let z = 0; return -z;"),
        ("positive_zero", "return 0 - 0;"),
        ("overflow", "return (2147483647 + 1) + 1;"),
        ("underflow_i32", "return (-2147483648 - 1) - 1;"),
        ("negate_min", "let a = -2147483648; return -a;"),
        ("unknown", "return x + 1;"),
        ("mutable", "let a = 1; a = 2.5; return a + 1;"),
        (
            "capture",
            "let a = 120; let f = () => a + 23; a = 121; return f();",
        ),
        ("multiply", "return 7 * 9;"),
        ("divide", "return 7 / 2;"),
        ("remainder", "return -7 % 3;"),
        ("nan", "return 0 / 0;"),
        ("infinity", "return 1 / 0;"),
        (
            "subnormal",
            "return 0.000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004;",
        ),
        ("effects", "return mark(1 + 2) - mark(4 + 5);"),
        (
            "trap_order",
            "let a = [0]; mark(1); a[2] = mark(2); return 0;",
        ),
    ] {
        cases.push(Case {
            name: name.to_owned(),
            source: format!("fn main(x: number) -> number {{ {body} }}"),
        });
    }
    cases
}
