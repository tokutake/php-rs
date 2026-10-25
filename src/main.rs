#![forbid(unsafe_code)]

mod array;
mod ast;
mod builtins;
mod diag;
mod interp;
mod lexer;
mod object;
mod parser;
mod value;

use std::io::{BufWriter, Write};
use std::process::ExitCode;

/// Reserved address space only; pages are committed as the stack actually grows
const INTERP_STACK_SIZE: usize = 1 << 30;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: php-rs <file.php>");
        return ExitCode::from(1);
    };
    let src = match std::fs::read(path) {
        Ok(s) => s,
        Err(_) => {
            println!("Could not open input file: {path}");
            return ExitCode::from(1);
        }
    };
    // Like php-cli, diagnostics show the resolved path of the script
    let file = std::fs::canonicalize(path).map_or_else(|_| path.clone(), |p| p.display().to_string());

    // PHP recursion maps onto native recursion in this tree-walking interpreter, so
    // run it on a thread whose stack fits interp::MAX_CALL_DEPTH nested calls
    let runner = std::thread::Builder::new().stack_size(INTERP_STACK_SIZE).spawn(move || run(&src, file));
    match runner.map(|t| t.join()) {
        Ok(Ok(true)) => ExitCode::SUCCESS,
        _ => ExitCode::from(255),
    }
}

fn run(src: &[u8], file: String) -> bool {
    let parsed = lexer::lex(src)
        .map_err(|e| ("Parse error", e.msg, e.line))
        .and_then(|tokens| parser::parse(tokens).map_err(|e| (e.label(), e.msg, e.line)));
    let program = match parsed {
        Ok(p) => p,
        Err((label, msg, line)) => {
            println!("\n{label}: {msg} in {file} on line {line}");
            // PHP 8.5 prints a backtrace for fatal errors, empty while compiling
            if label == "Fatal error" {
                println!("Stack trace:\n#0 {{main}}");
            }
            return false;
        }
    };
    let stdout = std::io::stdout().lock();
    let mut interp = interp::Interp::new(BufWriter::new(stdout), file);
    let ok = interp.run(&program);
    let _ = interp.into_output().flush();
    ok
}
