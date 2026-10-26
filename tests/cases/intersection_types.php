<?php
namespace Shapes;
interface A {}
interface B {}
interface C {}
class AB implements A, B {}
class OnlyA implements A {}
class OnlyC implements C {}
function take(A&B $value): A&B { return $value; }
function choose((A&B)|C|null $value): (B&A)|C|null { return $value; }
function refs(A&B &$value, A&B &...$rest): void { $value = $rest[0]; }
function many(A&B ...$values): int { return count($values); }
function attempt($f) { try { $f(); } catch (\Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; } }
$a = new AB;
var_dump(take($a) === $a, choose($a) === $a, choose(null), choose(new OnlyC) instanceof C);
attempt(fn() => take(new OnlyA));
attempt(fn() => take(1));
attempt(fn() => choose(new OnlyA));
function wrong(): A&B { return new OnlyA; }
attempt(fn() => wrong());
class Box {
    public A&B $value;
    public static (A&B)|C|null $shared;
    public function __construct(public A&B $promoted) {}
}
$b = new Box($a);
$b->value = $a;
Box::$shared = new OnlyC;
var_dump($b->value === $a, $b->promoted === $a, Box::$shared instanceof C);
attempt(function() use ($b) { $b->value = new OnlyA; });
attempt(function() { Box::$shared = new OnlyA; });
$replacement = new AB;
refs($a, $replacement);
var_dump($a === $replacement, many($a, $replacement));
attempt(fn() => many($a, new OnlyA));
$fn = fn(A&B $v): (A&B)|null => $v;
var_dump($fn($a) === $a);
function stream(): \Iterator&\Traversable { yield 1; }
var_dump(iterator_to_array(stream()));
namespace Other;
use Shapes\A as X;
use Shapes\B as Y;
function alias(X&Y $value): X&Y { return $value; }
var_dump(alias(new \Shapes\AB) instanceof X);
