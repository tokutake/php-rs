//! Inputs that crash or misbehave in naive implementations must end in a
//! regular PHP error, never a panic, stack overflow or runaway allocation.
//! Their output differs from php-src (which often runs out of memory instead),
//! so they are checked here rather than in the compat cases.

use std::process::Command;

fn run(name: &str, src: &str) -> (i32, String) {
    let dir = std::env::temp_dir().join(format!("php-rs-safety-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.php"));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_php-rs")).arg(&path).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

fn assert_php_error(name: &str, src: &str, expected: &str) {
    let (code, out) = run(name, src);
    assert!(!out.contains("panicked") && !out.contains("overflowed its stack"), "{name} crashed:\n{out}");
    assert_eq!(code, 255, "{name} should fail with a PHP error:\n{out}");
    assert!(out.contains(expected), "{name}: expected {expected:?} in:\n{}", &out[..out.len().min(2000)]);
}

#[test]
fn infinite_recursion_is_an_error() {
    assert_php_error("recursion", "<?php function r($n) { return r($n + 1); } r(0);", "Maximum call stack size reached");
}

#[test]
fn deep_nesting_is_a_parse_error() {
    let src = format!("<?php echo {}1{};", "[".repeat(1_000_000), "]".repeat(1_000_000));
    assert_php_error("nesting", &src, "Parse error: memory exhausted");
}

#[test]
fn writing_through_a_reference_cycle_is_an_error() {
    assert_php_error("refcycle", "<?php $x = [1]; $x[0] = &$x; $x[0][0] = 5;", "reference to itself");
}

#[test]
fn huge_allocations_are_refused() {
    assert_php_error("str_repeat", "<?php echo str_repeat('ab', PHP_INT_MAX);", "overflow in memory allocation");
    assert_php_error("offset", "<?php $s = 'a'; $s[PHP_INT_MAX] = 'b';", "String size overflow");
    assert_php_error("range", "<?php range(PHP_INT_MIN, PHP_INT_MAX);", "exceeds the maximum array size");
}

#[test]
fn integer_edge_cases_do_not_trap() {
    let (code, out) = run(
        "ints",
        "<?php var_dump(PHP_INT_MIN % -1, PHP_INT_MIN / -1, -PHP_INT_MIN, 1 << 64, PHP_INT_MAX + 1);",
    );
    assert_eq!(code, 0, "{out}");
    let expected = "int(0)\nfloat(9.223372036854776E+18)\nfloat(9.223372036854776E+18)\nint(0)\nfloat(9.223372036854776E+18)\n";
    assert_eq!(out, expected);
}

#[test]
fn inconsistent_comparisons_do_not_panic_sort() {
    let src = r#"<?php
        $a = [];
        for ($i = 0; $i < 3000; $i++) { $a[] = $i % 3 == 0 ? "abc$i" : ($i % 3 == 1 ? $i : "$i.5x"); }
        sort($a);
        function cmp($x, $y) { return rand_like($x, $y); }
        function rand_like($x, $y) { return (strlen("$x$y") % 3) - 1; }
        usort($a, 'cmp');
        echo count($a);
    "#;
    let (code, out) = run("sort", src);
    assert_eq!((code, out.as_str()), (0, "3000"));
}
