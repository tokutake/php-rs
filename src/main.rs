#![forbid(unsafe_code)]

mod array;
mod ast;
mod builtins;
mod diag;
mod datetime;
mod genlower;
mod interp;
mod lexer;
mod mysql;
mod object;
mod parser;
mod regex;
mod stream;
mod value;

use std::io::{BufWriter, Write};
use std::process::ExitCode;

/// Reserved address space only; pages are committed as the stack actually grows
const INTERP_STACK_SIZE: usize = 1 << 30;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    // `-l`: check the syntax only, as php -l does
    let lint = args.get(1).is_some_and(|a| a == "-l");
    if lint {
        args.remove(1);
    }
    // `--cgi`: the output starts with the header lines the script set, then a blank line
    let cgi = args.get(1).is_some_and(|a| a == "--cgi");
    if cgi {
        args.remove(1);
    }
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
    // $argv: the script as given, then its arguments
    let argv = args[1..].to_vec();
    let runner = std::thread::Builder::new().stack_size(INTERP_STACK_SIZE).spawn(move || run(&src, file, argv, lint, cgi));
    match runner.map(|t| t.join()) {
        Ok(Ok(status)) => ExitCode::from(status),
        _ => ExitCode::from(255),
    }
}

/// Runs a script; the exit status
fn run(src: &[u8], file: String, argv: Vec<String>, lint: bool, cgi: bool) -> u8 {
    let parsed = lexer::lex(src, &file)
        .map_err(|e| ("Parse error", e.msg, e.line))
        .and_then(|tokens| parser::parse(tokens, &file).map_err(|e| (e.label(), e.msg, e.line)));
    let program = match parsed {
        Ok(p) => p,
        Err((label, msg, line)) => {
            println!("\n{label}: {msg} in {file} on line {line}");
            // PHP 8.5 prints a backtrace for fatal errors, empty while compiling
            if label == "Fatal error" {
                println!("Stack trace:\n#0 {{main}}");
            }
            if lint {
                println!("Errors parsing {file}");
            }
            return 255;
        }
    };
    if lint {
        println!("No syntax errors detected in {file}");
        return 0;
    }
    if cgi {
        // The body is held back until the end, since headers set after output still count
        let mut interp = interp::Interp::new(Vec::new(), program.file.clone());
        interp.set_globals(&argv);
        let status = interp.run(&program);
        let mut head = Vec::new();
        let (code, headers) = interp.response_head();
        if let Some(code) = code {
            let _ = writeln!(head, "Status: {code}");
        }
        for line in headers {
            let _ = writeln!(head, "{line}");
        }
        let _ = writeln!(head);
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(&head).and_then(|()| stdout.write_all(&interp.into_output())).and_then(|()| stdout.flush());
        return status;
    }
    let stdout = std::io::stdout().lock();
    let mut interp = interp::Interp::new(BufWriter::new(stdout), program.file.clone());
    interp.set_globals(&argv);
    let status = interp.run(&program);
    let _ = interp.into_output().flush();
    status
}
