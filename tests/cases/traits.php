<?php
trait Greets {
    public $greeting = "Hello";
    private static $count = 0;
    public function greet($name) {
        self::$count++;
        return "$this->greeting, $name from " . __CLASS__ . " (" . __TRAIT__ . ", " . __METHOD__ . ", " . static::class . ")";
    }
    public static function count() { return self::$count; }
    abstract public function id(): string;
}
trait Logs {
    protected array $log = [];
    public function log(string $m): static { $this->log[] = $m; return $this; }
    public function greet($name) { return "log greet $name"; }
    public function dump() { return implode(",", $this->log); }
}
class Base {
    public function greet($name) { return "base $name"; }
    public function hello() { return "base hello"; }
}
class Person extends Base {
    use Greets, Logs {
        Greets::greet insteadof Logs;
        Logs::greet as protected logGreet;
        dump as public dumpLog;
    }
    public $own = 1;
    public function id(): string { return "person"; }
    public function both($n) { return $this->logGreet($n); }
}
class Robot {
    use Greets;
    public $greeting = "Hello";
    public function id(): string { return "robot"; }
}
$p = new Person;
echo $p->greet("Ann"), "\n";
echo $p->both("Bob"), "\n";
echo $p->log("a")->log("b")->dumpLog(), " ", $p->dump(), "\n";
echo $p->hello(), " ", $p->id(), "\n";
$r = new Robot;
echo $r->greet("Cy"), "\n";
echo Person::count(), " ", Robot::count(), "\n";
var_dump($p);
var_dump($p instanceof Greets, class_uses($p), method_exists($p, 'logGreet'));
try {
    $p->logGreet("x");
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
try {
    new Greets;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
// Traits using traits, static and abstract methods
trait Counter {
    private int $n = 0;
    public function inc(): int { return ++$this->n; }
}
trait Named {
    use Counter;
    public static function make(): static { return new static(); }
    public function name(): string { return static::class . "#" . $this->inc(); }
}
final class Widget { use Named; }
$w = Widget::make();
echo $w->name(), " ", $w->name(), "\n";
var_dump(trait_exists('Named'), class_exists('Named'));
// Overriding a trait method in the class; parent:: from a trait method
trait Hello {
    public function hi() { return "trait hi, " . parent::hi(); }
}
class P0 { public function hi() { return "P0 hi"; } }
class C1 extends P0 { use Hello; }
class C2 extends P0 { use Hello; public function hi() { return "own hi"; } }
echo (new C1)->hi(), " / ", (new C2)->hi(), "\n";
// Constants in traits
trait WithConst { public const VERSION = 2; }
class Versioned { use WithConst; }
echo Versioned::VERSION, "\n";
