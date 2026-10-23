<?php
function t($c) { try { var_dump($c()); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } }
// Built-in int parameters
t(fn() => intdiv("5 apples", 1));
t(fn() => intdiv(" 5", 1));
t(fn() => intdiv("5.0", 1));
t(fn() => intdiv(5.5, 1));
t(fn() => intdiv(null, 1));
t(fn() => intdiv(true, 1));
t(fn() => intdiv(1e20, 1));
t(fn() => intdiv([], 1));
// Built-in string parameters
t(fn() => strlen(5));
t(fn() => strlen(1.5));
t(fn() => strlen(true));
t(fn() => strlen(null));
t(fn() => strlen([]));
t(fn() => strlen(new Exception("msg")));
t(fn() => strlen(fn() => 1));
t(fn() => in_array(1, "x"));
t(fn() => explode(",", null));
t(fn() => implode(",", [1, [2]]));
// Arithmetic operands
t(fn() => "" + 1);
t(fn() => "abc" * 1);
t(fn() => -"abc");
t(fn() => "abc" % 2);
t(fn() => "abc" | 1);
t(fn() => "abc" << 1);
t(fn() => "abc" ** 1);
t(fn() => 1 / "x");
t(fn() => "5 apples" + 1);
t(fn() => (fn() => 1) + 1);
t(fn() => array_sum(["abc", 1, [2], 3.5]));
// User function parameters (non-strict mode)
function i(int $v) { return $v; }
function f(float $v) { return $v; }
function s(string $v) { return $v; }
function b(bool $v) { return $v; }
function n(?int $v) { return $v; }
function u(int|float $v) { return $v; }
function o(object $v) { return get_class($v); }
function e(Exception $v) { return $v->getMessage(); }
t(fn() => i("12"));
t(fn() => i("1.0"));
t(fn() => i("1.5"));
t(fn() => i(1.9));
t(fn() => i(null));
t(fn() => i(true));
t(fn() => f("3"));
t(fn() => f(7));
t(fn() => s(12));
t(fn() => s(1.5));
t(fn() => s(false));
t(fn() => s(new LogicException("as string")));
t(fn() => b("0"));
t(fn() => b([]));
t(fn() => n(null));
t(fn() => u("1.5"));
t(fn() => u("2"));
t(fn() => o(new Exception));
t(fn() => o("x"));
t(fn() => e(new RuntimeException("sub")));
t(fn() => e(new TypeError("no")));
