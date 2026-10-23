<?php
interface HasArea { const UNITS = "cm"; public function area(): float; }
interface Named { public function name(): string; }

abstract class Shape implements HasArea, Named {
    public static int $created = 0;
    protected static array $registry = [];
    const KIND = "shape";

    public function __construct(protected string $label = "unnamed") {
        static::$created++;
        self::$registry[] = static::class;
    }
    abstract public function area(): float;
    public function name(): string { return $this->label; }
    public function describe(): string {
        return sprintf_like(static::KIND, $this->name(), $this->area(), self::UNITS);
    }
    public static function make(...$args): static { return new static(...$args); }
    public static function registry(): array { return self::$registry; }
    public function __toString(): string { return static::class . "(" . $this->label . ")"; }
}

function sprintf_like($kind, $name, $area, $units) { return "$kind $name: $area $units"; }

class Rect extends Shape {
    const KIND = "rect";
    public function __construct(private float $w = 1, private float $h = 1, string $label = "rect") {
        parent::__construct($label);
    }
    public function area(): float { return $this->w * $this->h; }
    public function scale(float $f): static { $this->w *= $f; $this->h *= $f; return $this; }
}

final class Square extends Rect {
    const KIND = "square";
    public function __construct(float $side = 1) { parent::__construct($side, $side, "sq$side"); }
}

$r = new Rect(2, 3);
$s = Square::make(4);
echo $r->describe(), "\n", $s->describe(), "\n";
echo $s->scale(0.5)->scale(3)->area(), "\n";
echo "$r and $s\n";
echo Shape::$created, " ", count(Shape::registry()), " ", implode(",", Shape::registry()), "\n";
echo Rect::KIND, " ", Square::KIND, " ", Square::UNITS, " ", HasArea::UNITS, " ", Square::class, " ", $s::class, "\n";
var_dump($s instanceof Square, $s instanceof Rect, $s instanceof Shape, $s instanceof HasArea, $s instanceof Named, $r instanceof Square);
var_dump(get_class($s), get_parent_class($s), method_exists($s, 'area'), method_exists('Rect', 'nope'), property_exists('Rect', 'w'), property_exists($s, 'label'));
var_dump(class_exists('Rect'), class_exists('HasArea'), interface_exists('HasArea'), is_a($s, 'Shape'), is_subclass_of($s, 'Square'), is_subclass_of($s, 'Rect'));

// Dumps
var_dump($r);
print_r($s); echo "\n";
var_dump((array)$r);

// Objects are handles; clone copies
class Counter {
    public int $n = 0;
    public array $log = [];
    public ?Counter $child = null;
    public function inc(): static { $this->n++; $this->log[] = $this->n; return $this; }
    public function __clone() { $this->log[] = "cloned"; if ($this->child) { $this->child = clone $this->child; } }
}
$a = new Counter;
$b = $a;
$b->inc()->inc();
$c = clone $a;
$c->inc();
var_dump($a->n, $c->n, $a->log, $c->log);
$a->child = new Counter;
$d = clone $a;
$d->child->inc();
var_dump($a->child->n, $d->child->n, $a == $b, $a === $b, $a == $c, new Counter == new Counter);

// foreach over an object sees the properties visible from the current scope
class Visible {
    public $pub = 1;
    protected $prot = 2;
    private $priv = 3;
    public function inside() { $out = []; foreach ($this as $k => $v) { $out[] = "$k=$v"; } return implode(",", $out); }
}
$v = new Visible;
$v->dyn = 4;
foreach ($v as $k => $val) { echo "$k=$val "; }
echo "| ", $v->inside(), "\n";
var_dump(get_object_vars($v));

// stdClass and (object)
$o = new stdClass;
$o->x = 1;
$o->list[] = "a";
$o->list[] = "b";
$cast = (object)['name' => 'Ann', 'age' => 30];
var_dump($o, $cast->name, isset($cast->age), isset($cast->nope), (array)$cast);

// Typed properties
class Typed {
    public int $i = 0;
    public ?string $s = null;
    public float $f = 0.0;
    public int|string $u = 0;
    public array $arr = [];
}
$t = new Typed;
$t->i = "42";
$t->f = 3;
$t->s = 12;
$t->u = "x";
$t->arr[] = 1;
$t->i += 1;
$t->i++;
var_dump($t);

// Readonly and promoted properties
final class Point {
    public function __construct(public readonly int $x, public readonly int $y = 0) {}
    public function withX(int $x): static { return new static($x, $this->y); }
}
$p = new Point(1, 2);
$p2 = $p->withX(10);
var_dump($p, $p2->x);

// Closures and callables see $this and the class scope
class Sorter {
    private array $items;
    public function __construct(array $items) { $this->items = $items; }
    private function cmp($a, $b) { return $a <=> $b; }
    public static function rcmp($a, $b) { return $b <=> $a; }
    public function sorted() { $x = $this->items; usort($x, [$this, 'cmp']); return $x; }
    public function reversed() { $x = $this->items; usort($x, 'Sorter::rcmp'); return $x; }
    public function mapped() { return array_map(fn($v) => $v * count($this->items), $this->items); }
    public function getter() { return function () { return $this->items; }; }
}
$so = new Sorter([3, 1, 2]);
echo implode(",", $so->sorted()), " ", implode(",", $so->reversed()), " ", implode(",", $so->mapped()), " ", implode(",", ($so->getter())()), "\n";
var_dump(array_map([new Rect(1, 1), 'scale'], [2])[0]->area(), call_user_func([Shape::class, 'registry'])[0]);

// Static properties are shared through inheritance unless redeclared
class Base1 { public static $shared = 'base'; public static $own = 'base'; }
class Child1 extends Base1 { public static $own = 'child'; }
Child1::$shared = 'changed';
var_dump(Base1::$shared, Base1::$own, Child1::$own);
$cls = 'Child1';
var_dump($cls::$own, (new $cls) instanceof Base1);
