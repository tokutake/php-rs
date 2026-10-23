<?php
function t($c) { try { var_dump($c()); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } }
abstract class A {
    public int $typed;
    protected $prot = 1;
    private $priv = 2;
    public readonly int $ro;
    const PUB = 1;
    private const SECRET = 2;
    public static $s = 1;
    private static $ps = 2;
    public function __construct() { $this->ro = 5; }
    protected function protMethod() { return "prot"; }
    private function privMethod() { return "priv"; }
    public function inst() { return "inst"; }
    abstract public function abs();
}
class B extends A {
    public function abs() { return parent::protMethod(); }
    public function callPriv() { return $this->privMethod(); }
    public function readPriv() { return $this->priv; }
}
interface I {}
$b = new B;
t(fn() => $b->typed);
t(fn() => $b->prot);
t(fn() => $b->priv);
t(fn() => $b->ro = 6);
t(function () { $x = new B; unset($x->ro); });
t(fn() => $b->protMethod());
t(fn() => $b->privMethod());
t(fn() => $b->callPriv());
t(fn() => $b->readPriv());
t(fn() => $b->abs());
t(fn() => new A);
t(fn() => new I);
t(fn() => new Nope);
t(fn() => B::inst());
t(fn() => B::nope());
t(fn() => $b->nope());
t(fn() => B::SECRET);
t(fn() => B::NOPE);
t(fn() => B::$ps);
t(fn() => B::$nope);
t(fn() => $b->undefinedProp);
t(function () use ($b) { $b->typed = "abc"; });
t(function () use ($b) { $b->typed = 5; $b->typed .= "x"; });
t(function () use ($b) { $b->dynamic = 1; return $b->dynamic; });
t(fn() => (string)$b);
t(fn() => $b . "");
t(fn() => clone new Exception("x"));
t(fn() => $b->typed[] = 1);
t(function () { $n = null; $n->prop = 1; });
t(function () { $n = null; return $n->prop; });
t(fn() => (null)->method());
t(fn() => self::class);
t(fn() => new Closure);
t(function () { $x = 5; return $x::class; });
class Ro { public function __construct(public readonly array $items) {} }
t(function () { $r = new Ro([1]); $r->items[] = 2; });
t(function () { $r = new Ro([1]); $r->items = [2]; });
class Typed2 { public static ?int $count = null; }
t(function () { Typed2::$count = "7"; return Typed2::$count; });
t(function () { Typed2::$count = "seven"; });
