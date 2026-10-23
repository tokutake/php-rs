<?php
// Declarations are hoisted
echo early(), "\n";
function early() { return "hoisted"; }

function add(int $a, int $b = 10): int { return $a + $b; }
function greet(string $name, ?string $title = null) { return ($title ?? "Mx") . " $name"; }
function fact($n) { return $n <= 1 ? 1 : $n * fact($n - 1); }
function addOne(array &$arr) { $arr[] = 1; }
function counter() { static $n = 0; return ++$n; }
function sum(...$nums) { return array_sum($nums); }
function noret() {}
function v(): void { return; }
function fl(float $f) { return $f; }
function un(int|string $x) { return $x; }
function b(bool $b) { return $b; }
$g = 5;
function useGlobal() { global $g; $g++; return $g; }
var_dump(add(1), add(1, 2), add("5", 2.0), greet("Bob"), greet("Ann", "Dr"), fact(20), fact(21));
var_dump(fl(5), fl("1.5"), un(1.0), un("x"), un(true), b(0), b("a"));
$x = []; addOne($x); addOne($x); var_dump($x);
counter(); counter(); var_dump(counter());
var_dump(sum(), sum(1, 2, 3), sum(...[4, 5]), noret(), v());
var_dump(useGlobal(), $g);

// References
$a = 1; $b = &$a; $b = 2; var_dump($a);
unset($b); $b = 3; var_dump($a);
$arr = [1, 2, 3]; foreach ($arr as &$v) { $v *= 2; } unset($v); var_dump($arr);
$arr2 = [1, 2, 3]; foreach ($arr2 as &$v) {} foreach ($arr2 as $v) {} var_dump($arr2);
$data = ['a' => ['n' => 1]]; $ref = &$data['a']['n']; $ref = 99; $copy = $data; $ref = 100; var_dump($data, $copy);
function swap(&$p, &$q) { $t = $p; $p = $q; $q = $t; }
$m = 'left'; $n = 'right'; swap($m, $n); echo "$m $n\n";
$list = [3, 1, 2]; swap($list[0], $list[2]); echo implode(",", $list), "\n";
function &noRefReturn() { static $s = 0; return $s; }
$cyc = [1]; $cyc[] = &$cyc; var_dump($cyc); print_r($cyc); echo "\n";

// Callables
function cmp($a, $b) { return $a <=> $b; }
function double($x) { return $x * 2; }
function isOdd($x) { return $x % 2 == 1; }
$nums = [5, 3, 8, 1];
usort($nums, 'cmp'); echo implode(",", $nums), "\n";
$assoc = ['x' => 3, 'y' => 1, 'z' => 2];
uasort($assoc, 'cmp'); var_dump($assoc);
uksort($assoc, 'cmp'); var_dump(array_keys($assoc));
var_dump(array_map('double', ['a' => 1, 'b' => 2]), array_map(null, [1, 2], ['x']), array_map('strlen', ['hi', 'hello']));
var_dump(array_filter([1, 2, 3, 4, 5], 'isOdd'), array_filter([0, 1, '', 'a', null]));
var_dump(call_user_func('add', 1, 2), call_user_func_array('greet', ['Zed', 'Prof']), function_exists('add'), function_exists('strlen'), function_exists('nope'), is_callable('cmp'));
function args() { return [func_num_args(), func_get_args()]; }
var_dump(args(1, 'two'));

// Errors inside functions carry a stack trace
function inner($i, $s) { if ($i == 0) { return 1 % 0; } return inner($i - 1, $s); }
function outer() { return inner(2, "a long string argument here", [1]); }
outer();
