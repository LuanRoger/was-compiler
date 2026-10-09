use super::*;

fn parsed(args: &[&str]) -> Options {
    parse(args.iter().map(OsString::from)).unwrap()
}

#[test]
fn format_and_destination_matrix_preserves_exact_lowercase_extension_inference() {
    for (path, inferred) in [
        (None, Format::Wat),
        (Some("-"), Format::Wat),
        (Some("./-"), Format::Wat),
        (Some("program.wat"), Format::Wat),
        (Some("program.wasm"), Format::Wasm),
        (Some("program.WASM"), Format::Wat),
        (Some("program.Wasm"), Format::Wat),
        (Some("program.wasm.gz"), Format::Wat),
        (Some(".wasm"), Format::Wat),
        (Some("folder.wasm/program"), Format::Wat),
    ] {
        for explicit in [None, Some(Format::Wat), Some(Format::Wasm)] {
            let mut args = vec!["input.was"];
            if let Some(path) = path {
                args.extend(["-o", path]);
            }
            if let Some(format) = explicit {
                args.extend([
                    "--emit",
                    if format == Format::Wasm {
                        "wasm"
                    } else {
                        "wat"
                    },
                ]);
            }
            let format = explicit.unwrap_or(inferred);
            let destination = path.filter(|path| *path != "-");
            let result = parse(args.iter().map(OsString::from));

            if format == Format::Wasm && destination.is_none() {
                let error = result.unwrap_err();
                assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
                assert_eq!(error.exit_code(), 2);
                assert!(error.to_string().contains(
                    "binary Wasm requires -o with a filesystem path; use --emit wat for stdout"
                ));
            } else {
                let options = result.unwrap();
                assert_eq!(options.source, PathBuf::from("input.was"));
                assert_eq!(options.output, path.map(PathBuf::from));
                assert_eq!(options.format, explicit);
                assert_eq!(output_format(&options), format);
                assert_eq!(
                    output_file(options.output.as_deref()),
                    destination.map(Path::new)
                );
            }
        }
    }
}

#[test]
fn only_an_exact_dash_output_path_means_stdout() {
    assert_eq!(output_file(None), None);
    assert_eq!(output_file(Some(Path::new("-"))), None);

    for path in ["./-", "-file", "folder/-", "program.wasm", "program.WASM"] {
        assert_eq!(output_file(Some(Path::new(path))), Some(Path::new(path)));
    }
}

#[test]
fn optimization_syntax_maps_to_copyable_compiler_options_without_changing_artifact_policy() {
    fn copy(value: CompilerOptions) -> (CompilerOptions, CompilerOptions) {
        (value, value)
    }

    for (number, attached, level) in [
        ("0", "-O0", OptimizationLevel::None),
        ("1", "-O1", OptimizationLevel::Basic),
        ("2", "-O2", OptimizationLevel::Standard),
        ("3", "-O3", OptimizationLevel::Aggressive),
    ] {
        for flags in [vec![attached], vec!["-O", number]] {
            let mut args = vec!["input.was", "-o", "program.wasm"];
            args.extend(flags);
            let options = parsed(&args);
            let compiler_options = CompilerOptions {
                optimization_level: options.level,
            };

            assert_eq!(options.level, level);
            assert_eq!(copy(compiler_options), (compiler_options, compiler_options));
            assert_eq!(
                Compiler::with_options(compiler_options).options(),
                &compiler_options
            );
            assert_eq!(options.exports, ExportOptions::All);
            assert_eq!(output_format(&options), Format::Wasm);
            assert_eq!(
                output_file(options.output.as_deref()),
                Some(Path::new("program.wasm"))
            );
        }
    }

    assert_eq!(parsed(&["input.was"]).level, OptimizationLevel::None);
}

#[test]
fn export_mapping_preserves_owned_selection_order_and_defers_layout_validation() {
    assert_eq!(parsed(&["input.was"]).exports, ExportOptions::All);
    assert_eq!(
        parsed(&["input.was", "--no-exports"]).exports,
        ExportOptions::None
    );
    let options = parsed(&[
        "input.was",
        "--export",
        "second",
        "--export",
        "first",
        "--export",
        "second",
    ]);

    assert_eq!(
        options.exports,
        ExportOptions::Only(vec!["second".into(), "first".into(), "second".into()])
    );
    assert_eq!(output_format(&options), Format::Wat);
    let mut compiler = Compiler::new();
    compiler.set_exports(options.exports);
    let program = compiler
        .analyze("fn first() -> void {} fn second() -> void {}")
        .unwrap();
    assert!(
        matches!(compiler.compile(&program), Err(was_compiler::CodegenError::DuplicateExportSelection { name }) if name == "second")
    );

    let options = parsed(&["input.was", "--export", "missing"]);
    let mut compiler = Compiler::new();
    compiler.set_exports(options.exports);
    assert!(
        matches!(compiler.compile(&program), Err(was_compiler::CodegenError::UnknownExport { name }) if name == "missing")
    );
}

#[test]
fn host_mapping_preserves_registration_order_and_separates_syntax_from_validation() {
    let options = parsed(&[
        "input.was",
        "--host",
        "ready=app.ready()->bool",
        "--host",
        "echo=env.echo(string, number[])->string",
        "--host",
        "invalid",
    ]);

    assert_eq!(
        options.hosts,
        [
            "ready=app.ready()->bool",
            "echo=env.echo(string, number[])->string",
            "invalid"
        ]
    );
    assert_eq!(options.exports, ExportOptions::All);
    assert_eq!(output_format(&options), Format::Wat);
    let mut compiler = Compiler::new();
    for declaration in &options.hosts[..2] {
        register_host(&mut compiler, declaration).unwrap();
    }
    assert_eq!(compiler.host_functions().functions()[0].name, "ready");
    assert_eq!(compiler.host_functions().functions()[1].name, "echo");
    let program = compiler.analyze("fn main() -> void {}").unwrap();
    register_host(&mut compiler, "later=env.later()->void").unwrap();
    assert_eq!(program.host_functions.len(), 2);
    assert_eq!(program.host_functions[0].name, "ready");
    assert_eq!(program.host_functions[1].name, "echo");
    assert!(register_host(&mut compiler, &options.hosts[2]).is_err());
    assert_eq!(compiler.host_functions().functions().len(), 3);
}

#[test]
fn host_export_and_level_flags_do_not_change_explicit_format_or_stdout_policy() {
    for exports in [vec![], vec!["--no-exports"], vec!["--export", "main"]] {
        let mut args = vec![
            "input.was",
            "-o",
            "-",
            "--emit",
            "wat",
            "-O3",
            "--host",
            "print=env.log(number)->void",
        ];
        args.extend(exports);
        let options = parsed(&args);

        assert_eq!(output_format(&options), Format::Wat);
        assert_eq!(output_file(options.output.as_deref()), None);
        assert_eq!(options.level, OptimizationLevel::Aggressive);
        assert_eq!(options.hosts, ["print=env.log(number)->void"]);
        assert_eq!(options.source, PathBuf::from("input.was"));
    }
}
