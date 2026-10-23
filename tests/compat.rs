//! Runs tests/cases/*.php with php-rs and compares against *.out recorded with the reference PHP.
//! Regenerate expected output with tests/regen.sh. Script paths are stripped from
//! the output on both sides so the .out files are location independent.

use std::path::Path;
use std::process::Command;

#[test]
fn compat_cases() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    let mut cases: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "php"))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no test cases in {}", dir.display());

    let mut failures = Vec::new();
    for php in &cases {
        let expected = std::fs::read(php.with_extension("out")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_php-rs")).arg(php).output().unwrap();
        let prefix = format!("{}/", dir.canonicalize().unwrap().display());
        let actual = String::from_utf8_lossy(&output.stdout).replace(&prefix, "");
        if actual.as_bytes() != expected {
            failures.push(format!(
                "--- {}\n[expected]\n{}\n[actual]\n{}\n[stderr]\n{}",
                php.file_name().unwrap().to_string_lossy(),
                String::from_utf8_lossy(&expected),
                actual,
                String::from_utf8_lossy(&output.stderr),
            ));
        }
    }
    assert!(failures.is_empty(), "{}/{} cases failed\n{}", failures.len(), cases.len(), failures.join("\n"));
}
