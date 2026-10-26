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

#[test]
fn recursive_include_is_an_error() {
    // The script includes itself; the include path holds the script's directory
    assert_php_error("self_include", "<?php include __FILE__;", "Maximum call stack size of");
}

#[test]
fn deep_json_is_handled() {
    // The decoder keeps its own stack, and the nested result is freed without recursion
    let src = r#"<?php
$n = 200000;
$v = json_decode(str_repeat('[', $n) . str_repeat(']', $n), true, 1 << 30);
$o = json_decode(str_repeat('{"a":', $n) . '1' . str_repeat('}', $n), false, 1 << 30);
var_dump(is_array($v), is_object($o), json_validate(str_repeat('[', $n) . str_repeat(']', $n), 1 << 30));
$a = [];
for ($i = 0; $i < $n; $i++) { $a = [$a]; }
var_dump(json_encode($a, 0, 1 << 30));
echo json_last_error_msg();
unset($v, $o, $a);"#;
    let (code, out) = run("deep_json", src);
    assert_eq!(
        (code, out.as_str()),
        (0, "bool(true)\nbool(true)\nbool(true)\nbool(false)\nMaximum stack depth exceeded")
    );
}

#[test]
fn pathological_regexes_are_bounded() {
    // Catastrophic backtracking, deep recursion and nesting end in preg
    // errors (or compile errors), not hangs or stack overflows
    let src = r#"<?php
var_dump(preg_match('/(a+)+$/', str_repeat('a', 40) . 'b'), preg_last_error_msg());
var_dump(preg_match('/^(\w+\s?)*$/', str_repeat('word ', 30) . '!'), preg_last_error_msg());
var_dump(preg_match('/a(?R)?b/', str_repeat('a', 200000) . str_repeat('b', 200000)), preg_last_error_msg());
var_dump(preg_match('/((?=a(?1)?))/', str_repeat('a', 100000)), preg_last_error_msg());
var_dump(preg_match('/(?R)/', 'a'), preg_last_error_msg());
var_dump(preg_match('/(?:a|b)*$/', str_repeat('ab', 3000000)), preg_last_error_msg());
var_dump(@preg_match('/' . str_repeat('(', 100000) . 'a' . str_repeat(')', 100000) . '/', 'a'));
var_dump(@preg_match('/(?:(?:(?:ab){1000}){1000}){1000}/', 'a'));"#;
    let (code, out) = run("regex", src);
    assert_eq!(
        (code, out.as_str()),
        (
            0,
            "bool(false)\nstring(25) \"Backtrack limit exhausted\"\n\
             bool(false)\nstring(25) \"Backtrack limit exhausted\"\n\
             bool(false)\nstring(25) \"Recursion limit exhausted\"\n\
             bool(false)\nstring(25) \"Recursion limit exhausted\"\n\
             bool(false)\nstring(14) \"Internal error\"\n\
             bool(false)\nstring(25) \"Backtrack limit exhausted\"\n\
             bool(false)\n\
             bool(false)\n"
        )
    );
}

#[test]
fn endless_streams_hit_the_memory_limit() {
    // Reading an endless source stops at php-src's default memory_limit
    // instead of growing until the process is killed
    assert_php_error(
        "dev_zero",
        "<?php $z = fopen('/dev/zero', 'r'); echo strlen(fread($z, 10)), \"\\n\"; stream_get_contents($z);",
        "Allowed memory size of 134217728 bytes exhausted",
    );
    assert_php_error("dev_zero_line", "<?php fgets(fopen('/dev/zero', 'r'));", "Allowed memory size");
    // A huge length is not allocated up front
    let (code, out) = run("fread_len", "<?php var_dump(strlen(fread(fopen(__FILE__, 'r'), PHP_INT_MAX)));");
    assert_eq!((code, out.as_str()), (0, "int(65)\n"));
}

#[test]
fn extreme_dates_do_not_panic() {
    let (code, out) = run("extreme_dates", r#"<?php
        date_default_timezone_set('UTC');
        foreach ([PHP_INT_MIN, PHP_INT_MAX] as $t) {
            $d = new DateTime('@0');
            $d->setTimestamp($t);
            if ($d->getTimestamp() !== $t || strlen($d->format('c W o')) === 0) exit(1);
        }
        foreach (['9223372036854775807 weeks', '-9223372036854775808 days ago',
                  '9999999999999999999999-01-01', '@-9223372036854775808.1',
                  str_repeat('1', 100000), '日本語'] as $input) {
            if (strtotime($input, 0) !== false) exit(2);
        }
        if (gmmktime(PHP_INT_MAX, 0, 0, 1, 1, PHP_INT_MAX) !== false) exit(3);
        $d = new DateTime('@0');
        try { $d->setISODate(PHP_INT_MAX, PHP_INT_MAX, PHP_INT_MAX); exit(4); }
        catch (DateRangeError $e) {}
        echo 'safe';
    "#);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out, "safe");
}

#[test]
fn hostile_date_formats_and_intervals_do_not_panic() {
    let (code, out) = run("date_format_interval_edges", r#"<?php
        date_default_timezone_set('UTC');
        foreach ([['!X-m-d', '+9223372036854775807-12-31'],
                  ['!U', '-9223372036854775809'],
                  ['!U', str_repeat('9', 100000)],
                  [str_repeat('Y', 1100000), '2000'],
                  ['!u', '99999999999999999']] as [$format, $text]) {
            if (DateTimeImmutable::createFromFormat($format, $text) !== false) exit(1);
        }
        foreach (['P9223372036854775808Y', 'P9223372036854775807W',
                  'P'.str_repeat('9',100000).'D'] as $spec) {
            try { new DateInterval($spec); exit(2); }
            catch (DateMalformedIntervalStringException $e) {}
        }
        $d = new DateTimeImmutable('@0');
        try { $d->add(new DateInterval('P9223372036854775807Y')); exit(3); }
        catch (DateRangeError $e) {}
        $i = new DateInterval('PT0S'); $i->f = INF;
        try { $d->add($i); } catch (DateRangeError $e) {}
        echo 'safe';
    "#);
    assert_eq!((code, out.as_str()), (0, "safe"));
}

#[test]
fn deeply_nested_anonymous_classes_are_bounded() {
    let src = format!("<?php $x = {}0{};", "new class (".repeat(6000), ") {}".repeat(6000));
    assert_php_error("anonymous_depth", &src, "memory exhausted");
}
