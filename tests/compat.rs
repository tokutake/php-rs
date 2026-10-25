//! Runs tests/cases/*.php with php-rs and compares against *.out recorded with the reference PHP.
//! Regenerate expected output with tests/regen.sh. Script paths are stripped from
//! the output on both sides so the .out files are location independent.

use std::path::Path;
use std::process::Command;

/// Removes `prefix` from `out`. Inside `string(N) "..."` dumps the length is
/// adjusted too, so that var_dump output of paths does not depend on the checkout
/// location. tests/strip.php does the same for the reference output
fn strip_prefix(out: &[u8], prefix: &[u8]) -> Vec<u8> {
    let mut res = Vec::with_capacity(out.len());
    let mut i = 0;
    while i < out.len() {
        if out[i..].starts_with(b"string(") {
            let digits = out[i + 7..].iter().take_while(|b| b.is_ascii_digit()).count();
            let open = i + 7 + digits;
            if digits > 0 && out[open..].starts_with(b") \"") {
                let len: usize = std::str::from_utf8(&out[i + 7..open]).unwrap().parse().unwrap();
                let start = open + 3;
                if let Some(body) = out.get(start..start + len) {
                    let body = replace(body, prefix);
                    res.extend_from_slice(format!("string({}) \"", body.len()).as_bytes());
                    res.extend_from_slice(&body);
                    i = start + len;
                    continue;
                }
            }
        }
        if out[i..].starts_with(prefix) {
            i += prefix.len();
        } else {
            res.push(out[i]);
            i += 1;
        }
    }
    res
}

fn replace(s: &[u8], prefix: &[u8]) -> Vec<u8> {
    let mut res = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with(prefix) {
            i += prefix.len();
        } else {
            res.push(s[i]);
            i += 1;
        }
    }
    res
}

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

    let prefix = format!("{}/", dir.canonicalize().unwrap().display());
    let mut failures = Vec::new();
    for php in &cases {
        let expected = std::fs::read(php.with_extension("out")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_php-rs")).arg(php).output().unwrap();
        let actual = strip_prefix(&output.stdout, prefix.as_bytes());
        if actual != expected {
            failures.push(format!(
                "--- {}\n[expected]\n{}\n[actual]\n{}\n[stderr]\n{}",
                php.file_name().unwrap().to_string_lossy(),
                String::from_utf8_lossy(&expected),
                String::from_utf8_lossy(&actual),
                String::from_utf8_lossy(&output.stderr),
            ));
        }
    }
    assert!(failures.is_empty(), "{}/{} cases failed\n{}", failures.len(), cases.len(), failures.join("\n"));
}
