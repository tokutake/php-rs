<?php
function counter($n) {
    for ($i = 1; $i <= $n; $i++) {
        yield $i;
    }
    return "done";
}
foreach (counter(3) as $k => $v) echo "$k => $v\n";
$g = counter(2);
var_dump($g instanceof Generator, $g instanceof Iterator, $g instanceof Traversable);
echo $g->current(), "\n";
$g->next();
echo $g->key(), " ", $g->current(), "\n";
$g->next();
var_dump($g->valid(), $g->current(), $g->key(), $g->getReturn());

echo "-- keys\n";
function keyed() {
    yield 'a' => 1;
    yield 2;
    yield 10 => 3;
    yield 4;
    yield 'b' => 5;
    yield 6;
}
foreach (keyed() as $k => $v) echo gettype($k) == "integer" ? $k : "'$k'", " => $v\n";

echo "-- send\n";
function logger() {
    $n = 0;
    while (true) {
        $msg = yield $n;
        if ($msg === null) break;
        echo "got $msg\n";
        $n++;
    }
    return $n;
}
$l = logger();
var_dump($l->current());
var_dump($l->send("a"));
var_dump($l->send("b"));
$l->next();
var_dump($l->valid(), $l->getReturn());

echo "-- send before start\n";
function echoer() { echo "start\n"; while (true) { $x = yield; echo "x=$x\n"; } }
$e = echoer();
$e->send(1);
$e->send(2);

echo "-- yield in expressions\n";
function expr() {
    $a = (yield 1) + (yield 2);
    echo "sum $a\n";
    echo "got " . (yield 3) . "\n";
    $arr = [yield 4, yield 5];
    print_r($arr);
    $x = yield 6 ?: "default";
    echo "x=$x\n";
    return strtoupper(yield 7);
}
$g = expr();
foreach ([10, 20, 30, 40, 50, null, "end"] as $s) {
    echo "current: ", $g->current(), "\n";
    $g->send($s);
}
var_dump($g->getReturn());

echo "-- yield from\n";
function inner() { $x = yield 1; echo "inner got $x\n"; yield 2; return "inner-ret"; }
function outer() {
    yield 0;
    $r = yield from inner();
    echo "inner returned $r\n";
    yield from [10, 20];
    yield from ['k' => 'v'];
    yield 3;
}
foreach (outer() as $k => $v) echo "$k: $v\n";
$o = outer();
$o->current(); $o->next(); $o->send("S");
echo $o->current(), "\n";

echo "-- nested loops and control\n";
function grid() {
    foreach ([1, 2, 3] as $i) {
        if ($i == 2) continue;
        foreach (['a', 'b', 'c'] as $j) {
            if ($j == 'c') break;
            yield "$i$j";
        }
    }
    $i = 0;
    do { yield "do$i"; } while (++$i < 2);
    switch (2) {
        case 1: yield "one";
        case 2: yield "two";
        case 3: yield "three"; break;
        default: yield "default";
    }
    while (true) {
        yield "w";
        if (true) break;
    }
}
echo implode(",", iterator_to_array(grid(), false)), "\n";

echo "-- methods and closures\n";
class Repo {
    private $items = ['x' => 1, 'y' => 2];
    function all() { foreach ($this->items as $k => $v) yield $k => $v * 10; }
    static function range($a, $b) { for ($i = $a; $i <= $b; $i++) yield $i; }
}
foreach ((new Repo)->all() as $k => $v) echo "$k=$v ";
echo "\n";
echo implode(" ", iterator_to_array(Repo::range(3, 5))), "\n";
$gen = function ($n) { while ($n > 0) yield $n--; };
echo implode(" ", iterator_to_array($gen(3))), "\n";
$arrow = fn() => yield 42;
foreach ($arrow() as $v) echo $v, "\n";

echo "-- lazy start\n";
function lazy() { echo "started\n"; yield 1; }
$z = lazy();
echo "created\n";
$z->current();
$z->current();
echo "-- generator with params and refs\n";
function gen_by_val(array $a) { foreach ($a as $v) yield $v; }
echo implode(",", iterator_to_array(gen_by_val([1, 2]))), "\n";
var_dump(iterator_count(counter(5)));
