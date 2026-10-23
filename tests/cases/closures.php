<?php
// Capture by value and by reference
$x = 1;
$byVal = function () use ($x) { return $x; };
$byRef = function () use (&$x) { return $x; };
$x = 2;
var_dump($byVal(), $byRef());

$counter = function () { static $n = 0; return ++$n; };
$counter(); $counter();
var_dump($counter());

$total = 0;
$add = function ($n) use (&$total) { $total += $n; };
$add(5); $add(10);
var_dump($total);

// Captured values are copied per call
$list = [1];
$push = function ($v) use ($list) { $list[] = $v; return count($list); };
var_dump($push(2), $push(3), count($list));

// Arrow functions capture automatically, by value
$factor = 3;
$times = fn($n) => $n * $factor;
$factor = 100;
var_dump($times(2));
$nested = fn($a) => fn($b) => $a + $b + $factor;
var_dump($nested(1)(2));

// Recursion through a captured reference
$fib = function ($n) use (&$fib) { return $n < 2 ? $n : $fib($n - 1) + $fib($n - 2); };
var_dump($fib(15));

// Closures with built-ins
var_dump(array_map(fn($v) => $v * 2, [1, 2, 3]));
var_dump(array_filter(['a' => 1, 'b' => 2, 'c' => 3], fn($v) => $v % 2));
var_dump(array_filter([1, 2, 3, 4], fn($k) => $k > 1, ARRAY_FILTER_USE_KEY));
$people = [['n' => 'Bob', 'a' => 30], ['n' => 'Al', 'a' => 25], ['n' => 'Cy', 'a' => 35]];
usort($people, fn($p, $q) => $p['a'] <=> $q['a']);
echo implode(",", array_map(fn($p) => $p['n'], $people)), "\n";
var_dump(call_user_func(fn(...$xs) => array_sum($xs), 1, 2, 3));
var_dump(call_user_func_array(function ($a, $b) { return "$a-$b"; }, ['x', 'y']));

// Calling values
$name = 'strtoupper_missing';
$len = 'strlen';
var_dump($len("hello"), (function () { return "iife"; })(), is_callable($times), is_callable('nope'));
var_dump($times->__invoke(5));

// Type declarations on closures
$typed = function (int $n): string { return "n=$n"; };
var_dump($typed("42"));
try { $typed("x"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
function apply(callable $f, $v) { return $f($v); }
var_dump(apply(fn($v) => $v . "!", "hi"), apply('strlen', "four"));
try { apply("not a function", 1); } catch (TypeError $e) { echo get_class($e), "\n"; }

// var_dump / print_r of closures
var_dump($byVal, $times);
print_r(fn(int $a, &$b, $c = 1) => 0);
echo "\n";
try { echo $byVal; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($byVal == $byVal, $byVal === $byVal, $byVal === $byRef, $byVal instanceof Closure);

// Nullsafe operator
$maybe = null;
var_dump($maybe?->getMessage(), $maybe?->getMessage(undefined_function()));

// Errors inside closures name them by file and line
$bad = function () { return 1 % 0; };
$bad();
