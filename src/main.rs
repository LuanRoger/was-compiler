mod cli;

fn main() -> std::process::ExitCode {
    cli::main(std::env::args_os().skip(1))
}
