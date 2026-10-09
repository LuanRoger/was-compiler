use std::{
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::{AtomicU64, Ordering},
};

use clap::{
    CommandFactory, Parser, ValueEnum,
    builder::{NonEmptyStringValueParser, OsStringValueParser, TypedValueParser},
    error::ErrorKind,
};
use was_compiler::{
    CompileError, Compiler, CompilerOptions, ExportOptions, OptimizationLevel, Type,
};

use was_compiler::diagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Wasm,
    Wat,
}

#[derive(Debug, Default)]
struct Options {
    source: PathBuf,
    output: Option<PathBuf>,
    format: Option<Format>,
    exports: ExportOptions,
    hosts: Vec<String>,
    level: OptimizationLevel,
}

#[derive(Debug, Parser)]
#[command(
    name = "was-compiler",
    about = "Compile the language to WebAssembly or plain WAT",
    color = clap::ColorChoice::Never,
    after_help = "SOURCE is a required UTF-8 source file.\nUse -- before a source path starting with -.\n\nBinary Wasm requires an output file.\nWAT remains plain text and defaults to stdout.\nExample host declaration: 'print=env.log_number(number)->void'\nRun a binary: node runtime/run.cjs program.wasm --entry main --args '[]'"
)]
struct Arguments {
    #[arg(
        value_name = "SOURCE",
        value_parser = path_parser(),
        help = "Required UTF-8 source file"
    )]
    source: PathBuf,

    #[arg(
        short = 'o',
        long,
        value_name = "PATH",
        value_parser = path_parser(),
        help = "Write an artifact; - means text stdout"
    )]
    output: Option<PathBuf>,

    #[arg(
        long = "emit",
        value_name = "FORMAT",
        value_enum,
        help = "Output format (default WAT; .wasm output implies Wasm)"
    )]
    format: Option<Format>,

    #[arg(
        long = "export",
        value_name = "NAME",
        value_parser = NonEmptyStringValueParser::new(),
        conflicts_with = "no_exports",
        help = "Export only selected user functions; repeat for more"
    )]
    selected_exports: Vec<String>,

    #[arg(
        long,
        conflicts_with = "selected_exports",
        help = "Export no user functions (memory ABI exports remain)"
    )]
    no_exports: bool,

    #[arg(
        long = "host",
        value_name = "DECL",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Register NAME=MODULE.FIELD(TYPES)->RESULT; repeatable"
    )]
    hosts: Vec<String>,

    #[arg(
        short = 'O',
        value_name = "LEVEL",
        value_parser = clap::value_parser!(u8).range(0..=3),
        help = "Optimization preset (-O0, -O1, -O2, -O3; all currently no-ops)"
    )]
    level: Option<u8>,
}

fn path_parser() -> impl TypedValueParser<Value = PathBuf> {
    OsStringValueParser::new().try_map(|value: OsString| {
        if value.is_empty() {
            Err("path cannot be empty")
        } else {
            Ok(PathBuf::from(value))
        }
    })
}

pub fn main(args: impl Iterator<Item = OsString>) -> ExitCode {
    let options = match parse(args) {
        Ok(options) => options,
        Err(error) => {
            let exit = ExitCode::from(error.exit_code() as u8);

            return match error.print() {
                Ok(()) => exit,
                Err(error) => fail(1, format!("CLI output I/O error: {error}")),
            };
        }
    };

    match compile(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(1, error),
    }
}

fn fail(code: u8, message: String) -> ExitCode {
    eprintln!("{message}");

    ExitCode::from(code)
}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Options, clap::Error> {
    let args =
        Arguments::try_parse_from(std::iter::once(OsString::from("was-compiler")).chain(args))?;
    let exports = if args.no_exports {
        ExportOptions::None
    } else if args.selected_exports.is_empty() {
        ExportOptions::All
    } else {
        ExportOptions::Only(args.selected_exports)
    };
    let options = Options {
        source: args.source,
        output: args.output,
        format: args.format,
        exports,
        hosts: args.hosts,
        level: match args.level.unwrap_or(0) {
            0 => OptimizationLevel::None,
            1 => OptimizationLevel::Basic,
            2 => OptimizationLevel::Standard,
            _ => OptimizationLevel::Aggressive,
        },
    };

    if output_format(&options) == Format::Wasm && output_file(options.output.as_deref()).is_none() {
        return Err(Arguments::command().error(
            ErrorKind::MissingRequiredArgument,
            "binary Wasm requires -o with a filesystem path; use --emit wat for stdout",
        ));
    }

    Ok(options)
}

fn output_format(options: &Options) -> Format {
    options.format.unwrap_or_else(|| {
        if options.output.as_deref().and_then(Path::extension) == Some(OsStr::new("wasm")) {
            Format::Wasm
        } else {
            Format::Wat
        }
    })
}

fn output_file(output: Option<&Path>) -> Option<&Path> {
    output.filter(|path| *path != Path::new("-"))
}

fn compile(options: Options) -> Result<(), String> {
    let format = output_format(&options);
    let mut compiler = Compiler::with_options(CompilerOptions {
        optimization_level: options.level,
    });

    compiler.set_exports(options.exports);

    for declaration in &options.hosts {
        register_host(&mut compiler, declaration)
            .map_err(|error| format!("host registration error in {declaration:?}: {error}"))?;
    }

    let (label, source) = load_source(&options.source)?;
    let program = compiler
        .analyze(&source)
        .map_err(|error| diagnostic::render(&label, &source, &error))?;
    let wasm = compiler
        .compile(&program)
        .map_err(|error| diagnostic::render(&label, &source, &CompileError::Codegen(error)))?;
    let bytes = match format {
        Format::Wasm => wasm,
        Format::Wat => wasmprinter::print_bytes(&wasm)
            .map_err(|error| format!("{label}: WAT serialization error: {error}"))?
            .into_bytes(),
    };

    match output_file(options.output.as_deref()) {
        Some(output) => {
            if let (Ok(input), Ok(output)) = (options.source.canonicalize(), output.canonicalize())
                && input == output
            {
                return Err(
                    "output path refers to the source file; refusing to overwrite source".into(),
                );
            }

            write_artifact(output, &bytes)
                .map_err(|error| format!("{}: output I/O error: {error}", output.display()))
        }
        None => io::stdout()
            .lock()
            .write_all(&bytes)
            .map_err(|error| format!("stdout: I/O error: {error}")),
    }
}

fn load_source(path: &Path) -> Result<(String, String), String> {
    fs::read_to_string(path)
        .map(|source| (path.display().to_string(), source))
        .map_err(|error| format!("{}: input I/O error: {error}", path.display()))
}

fn register_host(compiler: &mut Compiler, declaration: &str) -> Result<(), String> {
    let syntax = "expected NAME=MODULE.FIELD(TYPES)->RESULT";
    let (name, signature) = declaration.split_once('=').ok_or(syntax)?;
    let (import, signature) = signature.split_once('(').ok_or(syntax)?;
    let (params, result) = signature.split_once(")->").ok_or(syntax)?;
    let (module, field) = import.split_once('.').ok_or(syntax)?;
    let params = if params.trim().is_empty() {
        Vec::new()
    } else {
        params
            .split(',')
            .map(parse_type)
            .collect::<Result<Vec<_>, _>>()?
    };
    let result = parse_type(result)?;

    compiler
        .register_host_function(name.trim(), module.trim(), field.trim(), params, result)
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn parse_type(name: &str) -> Result<Type, String> {
    match name.trim() {
        "number" => Ok(Type::Number),
        "bool" => Ok(Type::Bool),
        "string" => Ok(Type::String),
        "number[]" => Ok(Type::NumberArray),
        "void" => Ok(Type::Void),
        name => Err(format!(
            "unknown type {name:?}; expected number, bool, string, number[], or void"
        )),
    }
}

fn write_artifact(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    for _ in 0..64 {
        let temporary = parent.join(format!(
            ".was-compiler-{}-{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let written = file.write_all(bytes).and_then(|()| file.sync_all());

        drop(file);

        let result = written.and_then(|()| fs::rename(&temporary, path));

        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }

        return result;
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a temporary output file",
    ))
}

#[cfg(test)]
#[path = "cli_contract_tests.rs"]
mod contract_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn options(args: &[&str]) -> Options {
        parse(args.iter().map(OsString::from)).unwrap()
    }

    #[test]
    fn clap_configuration_is_valid() {
        Arguments::command().debug_assert();
    }

    #[test]
    fn clap_parses_attached_values_and_optimization_levels() {
        let parsed = options(&[
            "input.was",
            "--output=program.wasm",
            "--emit=wasm",
            "--export=main",
            "--host=print=env.log_number(number)->void",
            "-O2",
        ]);

        assert_eq!(parsed.output, Some(PathBuf::from("program.wasm")));
        assert_eq!(parsed.format, Some(Format::Wasm));
        assert_eq!(parsed.exports, ExportOptions::Only(vec!["main".into()]));
        assert_eq!(parsed.hosts, vec!["print=env.log_number(number)->void"]);
        assert_eq!(parsed.level, OptimizationLevel::Standard);
        assert_eq!(
            options(&["input.was", "-O", "3"]).level,
            OptimizationLevel::Aggressive
        );
    }

    #[test]
    fn source_file_is_required_even_with_output_options() {
        for args in [vec![], vec!["--emit", "wat"], vec!["-o", "program.wasm"]] {
            let error = parse(args.into_iter().map(OsString::from)).unwrap_err();

            assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
            assert_eq!(error.exit_code(), 2);
            assert!(error.to_string().contains("<SOURCE>"));
        }
    }

    #[test]
    fn clap_help_uses_a_successful_exit_status() {
        let error = parse([OsString::from("--help")].into_iter()).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::DisplayHelp);
        assert_eq!(error.exit_code(), 0);
    }

    #[test]
    fn default_and_file_formats_keep_wat_stdout_safe() {
        assert_eq!(output_format(&options(&["input.was"])), Format::Wat);
        assert_eq!(
            output_format(&options(&["input.was", "-o", "program.wasm"])),
            Format::Wasm
        );
        assert_eq!(
            output_format(&options(&[
                "input.was",
                "-o",
                "program.wasm",
                "--emit",
                "wat"
            ])),
            Format::Wat
        );
        assert!(
            parse(
                ["input.was", "--emit", "wasm"]
                    .map(OsString::from)
                    .into_iter()
            )
            .is_err()
        );
        assert!(
            parse(
                ["input.was", "--emit", "wasm", "-o", "-"]
                    .map(OsString::from)
                    .into_iter()
            )
            .is_err()
        );
    }

    #[test]
    fn invalid_and_conflicting_options_are_rejected() {
        for args in [
            vec!["--unknown"],
            vec!["-o"],
            vec!["--host", "--export", "main"],
            vec!["--emit", "bad"],
            vec!["a", "b"],
            vec!["--no-exports", "--export", "main"],
            vec!["--export", "main", "--no-exports"],
            vec!["--no-exports", "--no-exports"],
            vec!["-O0", "-O3"],
            vec!["-o", "a", "-o", "b"],
        ] {
            assert!(parse(args.iter().map(OsString::from)).is_err(), "{args:?}");
        }

        assert_eq!(
            options(&["--", "-source.was"]).source,
            PathBuf::from("-source.was")
        );
        assert_eq!(
            options(&["input.was", "--export", "main", "--export", "helper"]).exports,
            ExportOptions::Only(vec!["main".into(), "helper".into()])
        );
    }

    #[test]
    fn host_declarations_reuse_registration_validation() {
        let mut compiler = Compiler::new();

        register_host(&mut compiler, "echo=env.echo(string, number[])->string").unwrap();
        register_host(&mut compiler, "ready=app.ready()->bool").unwrap();

        assert_eq!(
            compiler.host_functions().functions()[0].params,
            vec![Type::String, Type::NumberArray]
        );
        assert_eq!(
            compiler.host_functions().functions()[1].return_type,
            Type::Bool
        );

        for declaration in [
            "bad",
            "x=env.x(number,)->void",
            "x=env.x(void)->void",
            "x=env.x(unknown)->number",
            "x=.x()->void",
            "echo=env.echo()->void",
            "len=env.len()->number",
        ] {
            assert!(
                register_host(&mut compiler, declaration).is_err(),
                "{declaration}"
            );
        }
    }
}
