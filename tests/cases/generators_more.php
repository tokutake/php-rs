<?php
// Recursive delegation: walking a tree
function walk(array $tree, int $depth = 0) {
    foreach ($tree as $name => $children) {
        yield str_repeat("  ", $depth) . $name;
        if ($children) {
            yield from walk($children, $depth + 1);
        }
    }
}
$tree = ['a' => ['b' => [], 'c' => ['d' => []]], 'e' => []];
foreach (walk($tree) as $line) echo $line, "\n";

// Pipelines
function numbers() { $i = 0; while (true) yield $i++; }
function take($it, $n) { foreach ($it as $k => $v) { if ($n-- <= 0) return; yield $k => $v; } }
function map(callable $f, $it) { foreach ($it as $k => $v) yield $k => $f($v); }
function filter(callable $f, $it) { foreach ($it as $k => $v) if ($f($v)) yield $k => $v; }
echo implode(",", iterator_to_array(take(filter(fn($x) => $x % 3 == 0, map(fn($x) => $x * $x, numbers())), 5), false)), "\n";

// Generators in objects keep $this
class Tokenizer {
    private $pos = 0;
    public function __construct(private string $src) {}
    public function tokens() {
        while ($this->pos < strlen($this->src)) {
            $c = $this->src[$this->pos++];
            if ($c === ' ') continue;
            yield $this->pos => $c;
        }
    }
    public static function make(string $s): static { return new static($s); }
}
foreach (Tokenizer::make("a b  c")->tokens() as $pos => $tok) echo "$pos:$tok ";
echo "\n";

// finally with return, and return values through yield from
function withFinally() {
    try {
        yield 1;
        return "from try";
    } finally {
        echo "finally runs\n";
    }
}
$g = withFinally();
foreach ($g as $v) echo "v=$v\n";
echo $g->getReturn(), "\n";
function chain() {
    $a = yield from withFinally();
    $b = yield from (function () { yield 2; return "inner"; })();
    return "$a + $b";
}
$c = chain();
foreach ($c as $k => $v) echo "$k=$v\n";
echo $c->getReturn(), "\n";

// Exceptions thrown into a delegate reach the inner generator
function innerCatch() {
    while (true) {
        try {
            $x = yield;
            echo "inner received $x\n";
        } catch (Exception $e) {
            echo "inner caught ", $e->getMessage(), "\n";
            return "recovered";
        }
    }
}
function outerDelegate() {
    $r = yield from innerCatch();
    echo "outer got $r\n";
    yield "after";
}
$o = outerDelegate();
$o->current();
$o->send("hello");
echo $o->throw(new Exception("oops")), "\n";

// Generators with references to outer state
function accumulate(array &$log) {
    while (true) {
        $item = yield;
        if ($item === null) return count($log);
        $log[] = $item;
    }
}
$log = [];
$acc = accumulate($log);
$acc->current();
$acc->send("x");
$acc->send("y");
$acc->send(null);
print_r($log);
echo $acc->getReturn(), "\n";

// Conditions, short-circuits and match with yield
function conds() {
    if ((yield 'a') == 1) {
        echo "branch one\n";
    } elseif ((yield 'b') == 2) {
        echo "branch two\n";
    }
    $x = (yield 'c') && (yield 'd');
    var_dump($x);
    $y = (yield 'e') ?? (yield 'f');
    var_dump($y);
    $z = match (yield 'g') { 1 => yield 'h', default => 'other' };
    var_dump($z);
    $i = 0;
    while (($v = yield "w$i") !== null) { echo "loop $v\n"; $i++; }
    for ($j = yield 'init'; $j < 2; $j += yield 'step') echo "for $j\n";
}
$g = conds();
foreach ([0, 2, 0, null, 7, 1, 'H', 'p', 'q', null, 0, 1, 1] as $send) {
    if (!$g->valid()) break;
    echo "at ", $g->current(), "\n";
    $g->send($send);
}

// Unused generators, generators kept at shutdown
function cleanup($name) { try { yield 1; yield 2; } finally { echo "cleanup $name\n"; } }
$kept = cleanup("kept");
$kept->current();
$tmp = cleanup("tmp");
$tmp->current();
$tmp = null;
function keys() { yield 1.5 => 'float'; yield true => 'bool'; yield null => 'null'; yield [1] => 'array'; }
foreach (keys() as $k => $v) { echo gettype($k), " => $v\n"; }
var_dump(iterator_to_array((function () { yield 1.5 => 'a'; yield true => 'b'; yield null => 'c'; })()));
echo "end\n";
