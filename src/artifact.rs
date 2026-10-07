use std::{borrow::Cow, fmt::Write};

use was_compiler::{ExportOptions, Type, TypedProgram};
use wasm_encoder::{CustomSection, Encode, Section};

const INTERFACE_SECTION: &str = "was-compiler.interface";
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_PARAMETERS: usize = 1024;

pub fn package(
    mut wasm: Vec<u8>,
    program: &TypedProgram,
    exports: &ExportOptions,
) -> Result<Vec<u8>, String> {
    let metadata = interface(program, exports)?;
    let section = CustomSection {
        name: Cow::Borrowed(INTERFACE_SECTION),
        data: Cow::Borrowed(metadata.as_bytes()),
    };

    wasm.push(section.id());
    section.encode(&mut wasm);

    Ok(wasm)
}

fn interface(program: &TypedProgram, exports: &ExportOptions) -> Result<String, String> {
    let mut json = String::from("{\"version\":1,\"memoryAbi\":\"descriptor-v1\",\"exports\":[");
    let mut separator = "";

    for function in &program.functions {
        let selected = match exports {
            ExportOptions::All => true,
            ExportOptions::None => false,
            ExportOptions::Only(names) => names.contains(&function.name),
        };

        if !selected {
            continue;
        }

        if matches!(function.name.as_str(), "memory" | "__alloc") {
            return Err(format!(
                "export `{}` is reserved by the typed runtime; select another export or emit raw WAT",
                function.name
            ));
        }

        json.push_str(separator);
        separator = ",";
        json.push_str("{\"name\":");
        quoted(&mut json, &function.name);
        signature(
            &mut json,
            function.params.iter().map(|param| param.ty),
            function.return_type,
        )?;
        json.push('}');
    }

    json.push_str("],\"imports\":[");
    separator = "";

    for host in &program.host_functions {
        json.push_str(separator);
        separator = ",";
        json.push_str("{\"name\":");
        quoted(&mut json, &host.name);
        json.push_str(",\"module\":");
        quoted(&mut json, &host.module);
        json.push_str(",\"field\":");
        quoted(&mut json, &host.field);
        signature(&mut json, host.params.iter().copied(), host.return_type)?;
        json.push('}');
    }

    json.push_str("]}");

    if json.len() > MAX_METADATA_BYTES {
        return Err(format!(
            "interface metadata exceeds the {MAX_METADATA_BYTES}-byte typed-runtime limit"
        ));
    }

    Ok(json)
}

fn signature(
    json: &mut String,
    params: impl ExactSizeIterator<Item = Type>,
    result: Type,
) -> Result<(), String> {
    if params.len() > MAX_PARAMETERS {
        return Err(format!(
            "typed-runtime signatures support at most {MAX_PARAMETERS} parameters"
        ));
    }

    json.push_str(",\"params\":[");
    let mut separator = "";

    for param in params {
        json.push_str(separator);
        separator = ",";
        quoted(json, type_name(param));
    }

    json.push_str("],\"result\":");
    quoted(json, type_name(result));

    Ok(())
}

fn type_name(ty: Type) -> &'static str {
    match ty {
        Type::Number => "number",
        Type::Bool => "bool",
        Type::String => "string",
        Type::NumberArray => "number[]",
        Type::Void => "void",
    }
}

fn quoted(json: &mut String, value: &str) {
    json.push('"');

    for character in value.chars() {
        match character {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            '\n' => json.push_str("\\n"),
            '\r' => json.push_str("\\r"),
            '\t' => json.push_str("\\t"),
            '\u{08}' => json.push_str("\\b"),
            '\u{0c}' => json.push_str("\\f"),
            character if character <= '\u{1f}' => {
                write!(json, "\\u{:04x}", u32::from(character)).expect("writing JSON to a String");
            }
            character => json.push(character),
        }
    }

    json.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use was_compiler::Compiler;

    #[test]
    fn metadata_follows_source_order_and_the_owned_host_snapshot() {
        let mut compiler = Compiler::new();

        compiler
            .register_host_function(
                "echo",
                "e\"nv",
                "é\n\\\u{0000}",
                vec![Type::String],
                Type::String,
            )
            .unwrap();

        let program = compiler
            .analyze("fn hidden() -> void {} fn text() -> string { return echo(\"hello\"); }")
            .unwrap();

        compiler
            .register_host_function("later", "env", "later", vec![], Type::Void)
            .unwrap();

        assert_eq!(
            interface(&program, &ExportOptions::Only(vec!["text".into()])).unwrap(),
            "{\"version\":1,\"memoryAbi\":\"descriptor-v1\",\"exports\":[{\"name\":\"text\",\"params\":[],\"result\":\"string\"}],\"imports\":[{\"name\":\"echo\",\"module\":\"e\\\"nv\",\"field\":\"é\\n\\\\\\u0000\",\"params\":[\"string\"],\"result\":\"string\"}]}"
        );
    }

    #[test]
    fn packaging_appends_one_valid_custom_section_without_rewriting_the_module() {
        let compiler = Compiler::new();
        let program = compiler
            .analyze("fn main() -> number { return 7; }")
            .unwrap();
        let raw = compiler.compile(&program).unwrap();
        let packaged = package(raw.clone(), &program, &ExportOptions::All).unwrap();

        assert!(packaged.starts_with(&raw));
        wasmparser::Validator::new()
            .validate_all(&packaged)
            .unwrap();

        let sections: Vec<_> = wasmparser::Parser::new(0)
            .parse_all(&packaged)
            .filter_map(|payload| match payload.unwrap() {
                wasmparser::Payload::CustomSection(section) => Some(section.name().to_owned()),
                _ => None,
            })
            .collect();

        assert_eq!(sections, vec![INTERFACE_SECTION]);
    }

    #[test]
    fn no_user_exports_still_describes_all_imports() {
        let mut compiler = Compiler::new();

        compiler
            .register_host_function("tick", "env", "tick", vec![], Type::Void)
            .unwrap();

        let program = compiler.analyze("fn main() -> void {}").unwrap();
        let metadata = interface(&program, &ExportOptions::None).unwrap();

        assert!(metadata.contains("\"exports\":[]"));
        assert!(metadata.contains("\"name\":\"tick\""));
        assert!(!metadata.contains("\"name\":\"main\""));
    }
}
