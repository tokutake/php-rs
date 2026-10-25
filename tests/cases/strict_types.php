<?php
declare(strict_types=1);

function takesInt(int $x) { return $x; }
function takesFloat(float $x) { return $x; }
function takesString(string $s) { return $s; }
function takesBool(bool $b) { return $b; }
function takesNullable(?int $x) { return $x; }
function returnsInt($v): int { return $v; }
class S { function __toString(): string { return "S"; } }
class P { public int $n = 0; public static float $f = 0.0; }

function attempt(callable $f) {
    try {
        var_dump($f());
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
}
attempt(fn () => takesInt(5));
attempt(fn () => takesInt("5"));
attempt(fn () => takesInt(5.0));
attempt(fn () => takesInt(true));
attempt(fn () => takesFloat(5));
attempt(fn () => takesFloat("1.5"));
attempt(fn () => takesString(5));
attempt(fn () => takesString(new S));
attempt(fn () => takesBool(0));
attempt(fn () => takesNullable(null));
attempt(fn () => returnsInt("7"));
attempt(fn () => returnsInt(7));
attempt(function () { $p = new P; $p->n = "3"; return $p->n; });
attempt(function () { $p = new P; $p->n = 3; return $p->n; });
attempt(function () { P::$f = 2; return P::$f; });
// Built-ins
attempt(fn () => str_repeat("ab", "2"));
attempt(fn () => str_repeat(5, 2));
attempt(fn () => strtoupper(123));
attempt(fn () => str_repeat("x", 3));
// Callbacks are called by internal code, which coerces as usual
attempt(fn () => array_map(fn (int $x) => $x * 2, ["1", "2"]));
attempt(fn () => usort($GLOBALS['undefined'], fn () => 0));
