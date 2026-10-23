<?php
class Magic {
    private array $data = [];
    public $real = "real";
    public function __get($n) { echo "get $n\n"; return $this->data[$n] ?? null; }
    public function __set($n, $v) { echo "set $n\n"; $this->data[$n] = $v; }
    public function __isset($n) { echo "isset $n\n"; return isset($this->data[$n]); }
    public function __unset($n) { echo "unset $n\n"; unset($this->data[$n]); }
    public function __call($m, $a) { return "$m(" . implode(",", $a) . ")"; }
    public static function __callStatic($m, $a) { return "static $m(" . count($a) . ")"; }
    public function __invoke($x) { return $x * 2; }
}
$m = new Magic;
$m->a = 1;
var_dump($m->a, isset($m->a), isset($m->b), empty($m->a), $m->a ?? "def", $m->real);
$m->a += 5;
var_dump($m->a);
unset($m->a);
var_dump(isset($m->a));
var_dump($m->foo(1, 2), Magic::bar(1), $m(21), array_map($m, [1, 2]), is_callable($m));

// The magic method sees the real property instead of recursing into itself
class Lazy {
    public function __get($name) { echo "computing $name\n"; return strtoupper($name); }
}
$l = new Lazy;
var_dump($l->abc);

class Recursive {
    private $hidden = "h";
    public function __get($n) { return "via __get: " . $n; }
}
$r = new Recursive;
var_dump($r->hidden, $r->other);
