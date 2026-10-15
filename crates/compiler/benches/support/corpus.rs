use std::fmt;

#[derive(Clone, Copy, Debug)]
pub enum Workload {
    Scalar(usize),
    Calls(usize),
    Captures(usize),
}

pub const CASES: [Workload; 9] = [
    Workload::Scalar(1),
    Workload::Scalar(16),
    Workload::Scalar(64),
    Workload::Calls(1),
    Workload::Calls(16),
    Workload::Calls(64),
    Workload::Captures(1),
    Workload::Captures(16),
    Workload::Captures(64),
];

impl fmt::Display for Workload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scalar(size) => write!(f, "scalar_{size}"),
            Self::Calls(size) => write!(f, "calls_{size}"),
            Self::Captures(size) => write!(f, "captures_{size}"),
        }
    }
}

pub fn source(workload: Workload) -> String {
    let mut source = String::new();

    match workload {
        Workload::Scalar(size) => {
            source.push_str("fn main() -> number { let total = 0;\n");
            for index in 0..size {
                source.push_str(&format!("total += {index} * 1.5 - 0.25;\n"));
            }
            source.push_str("return total; }");
        }
        Workload::Calls(size) => {
            source.push_str("fn main() -> number { let total = 0;\n");
            for index in 0..size {
                source.push_str(&format!("total += step_{index}({index});\n"));
            }
            source.push_str("return total; }\n");
            for index in 0..size {
                source.push_str(&format!(
                    "fn step_{index}(x: number, extra: number = x + 1) -> number \
                     requires x >= 0 {{\n\
                     if x < 8 {{ return x + extra; }} else {{ return x - extra; }}\n\
                     }}\n"
                ));
            }
        }
        Workload::Captures(size) => {
            source.push_str("fn main() -> number { let total = 0; let values = [1, 2, 3];\n");
            for index in 0..size {
                source.push_str(&format!(
                    "let update_{index} = () => {{\n\
                     for x in 1..=3 {{ values[0] += x; }}\n\
                     for x in values {{ total += x; }}\n\
                     total\n\
                     }};\n\
                     total += update_{index}();\n"
                ));
            }
            source.push_str("return total; }");
        }
    }

    source
}
