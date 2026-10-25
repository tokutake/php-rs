<?php
class D {
    public function __construct(public string $name) { echo "ctor {$this->name}\n"; }
    public function __destruct() { echo "dtor {$this->name}\n"; }
}

echo "-- unset and reassign\n";
$a = new D("a");
unset($a);
echo "after unset\n";
$b = new D("b1");
$b = new D("b2");
echo "after reassign\n";
$b = null;
echo "after null\n";

echo "-- temporaries\n";
new D("temp");
echo "after temp\n";
function make($n) { $x = new D($n); echo "in make\n"; }
make("local");
echo "after make\n";
function ret() { return new D("returned"); }
echo ret()->name, "\n";
echo "after ret\n";

echo "-- shared\n";
$c = new D("c");
$d = $c;
unset($c);
echo "still alive\n";
$d = 1;
echo "-- arrays\n";
$arr = [new D("e0"), new D("e1")];
array_pop($arr);
echo "popped\n";
$arr = [];
echo "cleared\n";

echo "-- nested\n";
class Holder {
    public $child;
    public function __construct(public string $name) { $this->child = new D("child of $name"); }
    public function __destruct() { echo "dtor holder {$this->name}\n"; }
}
$h = new Holder("h");
$h = null;

echo "-- this in destructor\n";
class Counter {
    public static $live = 0;
    public function __construct() { self::$live++; }
    public function __destruct() { self::$live--; echo "live: ", self::$live, "\n"; var_dump($this); }
}
$x = new Counter; $y = new Counter;
$x = null;
echo "-- exception from destructor\n";
class Thrower { public function __destruct() { throw new Exception("from dtor"); } }
try {
    $t = new Thrower;
    unset($t);
    echo "not reached\n";
} catch (Exception $e) {
    echo "caught: ", $e->getMessage(), "\n";
}
echo "-- explicit call\n";
$e2 = new D("explicit");
$e2->__destruct();
echo "-- inheritance\n";
class Child extends D {}
$k = new Child("child");
unset($k);
class Child2 extends D { public function __destruct() { echo "child2 first\n"; parent::__destruct(); } }
$k = new Child2("child2");
unset($k);
echo "-- cycle\n";
$p = new D("cyc1"); $q = new D("cyc2");
$p->other = $q; $q->other = $p;
unset($p, $q);
echo "-- end\n";
$g1 = new D("g1");
$g2 = new D("g2");
$g3 = new D("g3");
$keep = $g2;
