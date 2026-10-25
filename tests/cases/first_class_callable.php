<?php
// First-class callable syntax (f(...)) and Closure::fromCallable()
function add(int $a, int $b = 10) { return $a + $b; }
class A {
    public function __construct(public $v = 1) {}
    public function m($x) { return $this->v * $x; }
    public static function s($x) { return static::class . $x; }
    private function priv() { return "priv"; }
    public function getPriv() { return $this->priv(...); }
    public function __call($n, $a) { return "__call $n " . implode(',', $a); }
    public static function __callStatic($n, $a) { return "__callStatic $n " . implode(',', $a); }
    public function thrower() { throw new Exception("t"); }
}
class B extends A {}
$f = add(...);
var_dump($f(1), $f(1, 2), $f(b: 5, a: 1));
$s = strlen(...);
var_dump($s("hello"));
$u = strtoupper(...);
var_dump(array_map($u, ['a', 'b']));
$a = new A(3);
$m = $a->m(...);
var_dump($m(2));
$st = B::s(...);
var_dump($st('!'));
var_dump($a->getPriv()());
$magic = $a->undefined(...);
var_dump($magic(1, 2));
$ms = A::undef(...);
var_dump($ms(3));
$inv = $f(...);
var_dump($inv(5));
var_dump($f instanceof Closure, $s instanceof Closure);
$c = Closure::fromCallable('add');
var_dump($c(2));
$c = Closure::fromCallable([$a, 'm']);
var_dump($c(4));
$c = Closure::fromCallable('A::s');
var_dump($c('?'));
$c = Closure::fromCallable(fn($x) => $x * 2);
var_dump($c(21));
$sorted = [3, 1, 2];
$sort = sort(...);
try { $t = $a->thrower(...); $t(); } catch (Exception $e) { echo $e->getTraceAsString(), "\n"; }
try { $x = nope(...); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $x = Nope::m(...); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { Closure::fromCallable('nope'); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $s(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $f(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $s([]); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$p = 'strrev';
$pf = $p(...);
var_dump($pf('abc'));
$arrc = [$a, 'm'];
var_dump(($arrc(...))(5));
function &refret(array &$arr) { $arr[] = 1; return $arr; }
function byref(array &$arr) { $arr[] = 1; }
$br = byref(...);
$z = [];
$br($z);
var_dump(count($z));
var_dump((new A(7))->m(...)(2));

echo "-- invalid callables\n";
class P2 { private function x() {} }
class R { public function inst() {} public static function st() {} }
foreach (['nope', [new P2, 'nope'], [new P2, 'x'], ['Nope', 'x'], [1], 5, 'R::nope', 'R::inst', 'Nope::x', [new R, 5], ['R', 'inst'], 'R::st'] as $cb) {
    try {
        var_dump(Closure::fromCallable($cb) instanceof Closure);
    } catch (Error $e) {
        echo $e->getMessage(), "\n";
    }
    try {
        call_user_func($cb);
        echo "called\n";
    } catch (Error $e) {
        echo $e->getMessage(), "\n";
    }
}
$x = 5;
try { $x(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $x(...); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { (new R)(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { (new R)(...); } catch (Error $e) { echo $e->getMessage(), "\n"; }
echo "end\n";
