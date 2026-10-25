<?php
// ArrayAccess and Countable: which methods run for each kind of access

class Coll implements ArrayAccess, Countable {
    private array $d = [];
    public function offsetExists(mixed $k): bool { echo "exists(", var_export($k, true), ")\n"; return isset($this->d[$k]); }
    public function offsetGet(mixed $k): mixed { echo "get(", var_export($k, true), ")\n"; return $this->d[$k] ?? null; }
    public function offsetSet(mixed $k, mixed $v): void {
        echo "set(", var_export($k, true), ")\n";
        if ($k === null) {
            $this->d[] = $v;
        } else {
            $this->d[$k] = $v;
        }
    }
    public function offsetUnset(mixed $k): void { echo "unset(", var_export($k, true), ")\n"; unset($this->d[$k]); }
    public function count(): int { return count($this->d); }
}

$c = new Coll;
$c['a'] = 1;
$c[] = 2;
var_dump($c['a']);
var_dump(isset($c['a']), isset($c['zz']), empty($c['a']), empty($c['zz']));
var_dump($c['a'] ?? 'default', $c['zz'] ?? 'default');
$c['a'] .= 'x';
$c['n'] = 5;
$c['n'] += 10;
var_dump($c['a'], $c['n']);
unset($c['a']);
var_dump(count($c), count($c, COUNT_RECURSIVE), $c instanceof ArrayAccess, $c instanceof Countable);

echo "-- indirect modification\n";
$c['arr'] = [1, 2];
$c['arr'][] = 3;
var_dump($c['arr']);
$v = $c['n']++;
var_dump($v, $c['n']);
unset($c['arr'][0]);
$r = &$c['ref'];
sort($c['arr']);
foreach ($c['arr'] as &$x) {
}
unset($x);
try {
    $c['r'] = &$y;
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

echo "-- nested objects\n";
$c['o'] = new Coll;
$c['o']['x'] = 5;
var_dump($c['o']['x']);
var_dump(isset($c['o']['x']), isset($c['o']['nope']));
$arr = ['c' => new Coll];
$arr['c']['z'] = 3;
var_dump($arr['c']['z']);

class Holder {
    public function __construct(public readonly Coll $c) {}
}
$h = new Holder(new Coll);
$h->c['k'] = 2;
var_dump($h->c['k']);

echo "-- ??= and destructuring\n";
$i = $c['q'] ??= 'filled';
var_dump($i);
$c2 = new Coll;
$c2[0] = 'zero';
$c2[1] = 'one';
[$p, $q] = $c2;
['missing' => $m] = $c2;
var_dump($p, $q, $m);

echo "-- offsets of any type\n";
class Echoer implements ArrayAccess {
    public function offsetExists($k): bool { var_dump($k); return true; }
    public function offsetGet($k): mixed { var_dump($k); return null; }
    public function offsetSet($k, $v): void { var_dump($k); }
    public function offsetUnset($k): void { var_dump($k); }
}
$e = new Echoer;
$e[new stdClass] = 'obj';
$e[true] = 't';
$e[1.5] = 'f';
$e[] = 'n';
$e[[1, 2]];
isset($e[null]);
unset($e['u']);

echo "-- evaluation order\n";
function f() { echo "f\n"; return 'k'; }
function g() { echo "g\n"; return 'm'; }
$c[f()][g()] = 1;

echo "-- errors\n";
$o = new stdClass;
foreach ([
    fn () => $o['x'] = 1,
    fn () => $o['x'],
    fn () => isset($o['x']),
    function () use ($o) { unset($o['x']); },
    fn () => count($o),
    fn () => $o['x']['y'] = 1,
] as $f) {
    try {
        $f();
    } catch (Error $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

class Thrower implements ArrayAccess {
    public function offsetExists($k): bool { return true; }
    public function offsetGet($k): mixed { throw new RuntimeException("no $k"); }
    public function offsetSet($k, $v): void {}
    public function offsetUnset($k): void {}
}
function read_it() {
    $t = new Thrower;
    return $t[5];
}
try {
    read_it();
} catch (RuntimeException $e) {
    echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n";
}

echo "-- plain arrays holding objects\n";
class P { public $d = []; function put($k, $v) { $this->d[$k] = $v; } }
$pp = new P;
$pp->put('o', new P);
$list = [];
$list['o'] = new P;
$list['o']->d['x'] = 1;
var_dump(count($pp->d), $list['o']->d);
class S { function __toString(): string { return "S"; } }
$str = "abc";
$str[1] = new S;
var_dump($str);
