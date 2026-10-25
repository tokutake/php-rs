<?php
class NumberList implements Iterator {
    private $i = 0;
    public function __construct(private array $items) {}
    public function current(): mixed { echo "[current]"; return $this->items[$this->i]; }
    public function key(): mixed { echo "[key]"; return $this->i; }
    public function next(): void { echo "[next]"; $this->i++; }
    public function rewind(): void { echo "[rewind]"; $this->i = 0; }
    public function valid(): bool { echo "[valid]"; return $this->i < count($this->items); }
}
foreach (new NumberList([10, 20]) as $k => $v) echo " $k=$v\n";
echo "\n";
foreach (new NumberList([1]) as $v) echo " $v\n";
echo "\n";
class GenBag implements IteratorAggregate {
    public function getIterator(): Generator { yield 'a' => 1; yield 'b' => 2; }
}
foreach (new GenBag as $k => $v) echo "$k=$v\n";
var_dump(new GenBag instanceof Traversable, new NumberList([]) instanceof Traversable);
print_r(iterator_to_array(new GenBag));
echo iterator_count(new NumberList([1, 2, 3])), "\n";
function wrap(iterable $it) { foreach ($it as $k => $v) yield $k => $v; }
print_r(iterator_to_array(wrap(new GenBag)));
print_r(iterator_to_array(wrap(['x' => 1])));
function delegate() { $r = yield from new GenBag; var_dump($r); yield from new NumberList([7]); }
foreach (delegate() as $k => $v) echo " $k=$v\n";
echo "\n";
class BadAgg implements IteratorAggregate { public function getIterator(): Traversable { return (function () { yield 1; })(); } }
foreach (new BadAgg as $v) echo $v, "\n";
function typed(): iterable { yield 1; }
function typed2(): Traversable { yield 2; }
function typed3(): mixed { yield 3; }
echo typed()->current(), typed2()->current(), typed3()->current(), "\n";
