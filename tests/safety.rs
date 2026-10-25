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
    assert_php_error("recursion", "<?php function r($n) { return r($n + 1); } r(0);", "Maximum call stack size of");
    assert_php_error(
        "tostring",
        "<?php class S { function __toString(): string { return (string)$this; } } echo new S;",
        "Maximum call stack size of",
    );
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

#[test]
fn self_referencing_objects_are_handled() {
    // Freeing a million-node linked list must not overflow the stack
    let src = r#"<?php
        class Node { public $next; }
        $head = null;
        for ($i = 0; $i < 1000000; $i++) { $n = new Node; $n->next = $head; $head = $n; }
        $head = $n = null;
        echo "freed";
    "#;
    assert_eq!(run("list", src), (0, "freed".to_string()));
    assert_php_error(
        "cmp",
        "<?php class A { public $me; } $x = new A; $x->me = $x; $y = new A; $y->me = $y; var_dump($x == $y);",
        "Nesting level too deep",
    );
    let (code, out) = run("dump", "<?php class A { public $me; } $x = new A; $x->me = $x; print_r($x);");
    assert_eq!(code, 0);
    assert!(out.contains("*RECURSION*"), "{out}");
}

#[test]
fn destructors_do_not_overflow_the_stack() {
    // Each node's destructor is due when the previous one is gone; php-src recurses here
    let src = r#"<?php
        class Node { public $next; public static $n = 0; function __destruct() { self::$n++; } }
        $head = null;
        for ($i = 0; $i < 1000000; $i++) { $n = new Node; $n->next = $head; $head = $n; }
        $head = $n = null;
        echo Node::$n;
    "#;
    assert_eq!(run("destruct_list", src), (0, "1000000".to_string()));
    // A destructor whose local dies with a fresh object in it: each one runs
    // after the previous frame is gone, but still nests
    assert_php_error(
        "destruct_chain",
        "<?php class R { function __destruct() { $keep = new R; } } new R;",
        "Maximum call stack size of",
    );
}

#[test]
fn deep_generator_delegation_is_an_error() {
    // Each level of `yield from` resumes the next one natively
    assert_php_error(
        "yield_from",
        "<?php function d($n) { if ($n) { yield from d($n - 1); } else { yield 1; } } foreach (d(100000) as $v) {}",
        "Maximum call stack size of",
    );
    assert_php_error(
        "gen_recursion",
        "<?php function r() { $g = r(); $g->current(); yield 1; } r()->current();",
        "Maximum call stack size of",
    );
}
