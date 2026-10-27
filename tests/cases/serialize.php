<?php
// Scalars and arrays
foreach ([null, true, false, 0, -42, PHP_INT_MAX, PHP_INT_MIN, 1.5, 0.1, -0.0, 1e100, 2.0, INF, -INF, NAN, "", "héllo\"\0x", [], [1, 2], ['a' => 1, 5 => [true]], [-3 => 'x', '07' => 'y']] as $v) {
    $s = serialize($v);
    echo $s, "\n";
    var_dump(unserialize($s));
}
// Objects and visibility
class P { public $a = 1; protected $b = [2]; private $c = 'three'; public $n; }
class C extends P { private $c = 'child'; public ?int $typed = null; public int $uninit; }
$c = new C; $c->dyn = 'dynamic';
echo $s = serialize($c), "\n";
var_dump(unserialize($s));
echo serialize(new stdClass), "\n";
$o = new stdClass; $o->x = 1; $o->{'0'} = 'zero'; echo $s = serialize($o), "\n"; var_dump(unserialize($s));
// Shared objects and references
$shared = new stdClass; $shared->v = 1;
$arr = [$shared, $shared, 'k' => $shared];
echo $s = serialize($arr), "\n";
$u = unserialize($s); $u[0]->v = 99; var_dump($u[1]->v, $u['k']->v);
$x = 1; $refs = [&$x, &$x, 2];
echo $s = serialize($refs), "\n";
$u = unserialize($s); $u[0] = 'changed'; var_dump($u);
$self = new stdClass; $self->me = $self; echo $s = serialize($self), "\n"; $u = unserialize($s); var_dump($u->me === $u);
$a = [1]; $a[] = &$a; echo serialize($a), "\n";
$or = new stdClass; $refo = [&$or, &$or, $or]; echo serialize($refo), "\n";
// Enums
enum Suit: string { case Hearts = 'H'; case Spades = 'S'; }
enum Pure { case One; }
echo $s = serialize([Suit::Hearts, Pure::One, Suit::Hearts]), "\n";
var_dump(unserialize($s), unserialize($s)[0] === Suit::Hearts);
// Magic methods
class M {
    public $a = 1; public $b = 2; private $secret = 's'; protected $p = 'p';
    function __sleep() { echo "sleep\n"; return ['a', 'secret', 'p']; }
    function __wakeup() { echo "wakeup ", $this->a, "\n"; $this->b = 'woken'; }
}
echo $s = serialize(new M), "\n"; var_dump(unserialize($s));
class S2 {
    public $data = [1, 2];
    function __serialize(): array { echo "__serialize\n"; return ['d' => $this->data, 7 => 'x']; }
    function __unserialize(array $d): void { echo "__unserialize ", json_encode($d), "\n"; $this->data = $d['d']; }
}
echo $s = serialize([new S2, new S2]), "\n"; var_dump(unserialize($s));
class Bad { function __sleep() { return ['nope', 'a', 'a']; } public $a = 1; }
echo serialize(new Bad), "\n";
class Bad2 { function __sleep() { return 5; } }
echo serialize(new Bad2), "\n";
class Bad3 { function __serialize() { return 5; } }
try { serialize(new Bad3); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { serialize(function () {}); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { unserialize('O:7:"Closure":0:{}'); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
// Incomplete classes and allowed_classes
$u = unserialize('O:7:"Missing":1:{s:1:"a";i:1;}'); var_dump($u, get_class($u), serialize($u));
var_dump(unserialize(serialize(new P), ['allowed_classes' => false]));
var_dump(unserialize(serialize([new P, new C]), ['allowed_classes' => ['c']]));
try { unserialize('i:1;', ['allowed_classes' => 1]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { unserialize('i:1;', ['max_depth' => -1]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(unserialize('a:1:{i:0;a:1:{i:0;a:0:{}}}', ['max_depth' => 2]));
// Malformed input
foreach (['', 'x', 'i:1', 'i:1;junk', 's:5:"abc";', 's:3:"abc"', 'a:2:{i:0;i:1;}', 'a:1:{i:0;i:1;i:1;i:2;}', 'b:2;', 'd:1.5e3;', 'i:99999999999999999999;', 'O:1:"A":1:{s:1:"a";}', 'R:1;', 'a:1:{i:0;R:5;}', 'r:1;', 'E:5:"Suit:";', 'E:9:"Suit:Nope";', 'E:6:"Nope:A";', 'E:4:"Suit";', 'N;N;', 'a:1:{i:0;O:8:"stdClass":0:{}}', 'a:1:{s:1:"1";i:5;}', 'i:+5;', 'i:-0;', 'd:.5;', 'd:5.;', 'S:3:"\61bc";', 'C:3:"Foo":3:{abc}', 'O:8:"stdClass":1:{i:5;s:1:"x";}', 'a:-1:{}', 'O:3:"Bad":1:{s:1:"a";i:1;}'] as $bad) {
    echo json_encode($bad), ": "; var_dump(unserialize($bad));
}
// References to properties and nested arrays
$o = new stdClass; $o->list = [1, 2]; $o->alias = &$o->list;
echo $s = serialize($o), "\n"; $u = unserialize($s); $u->alias[] = 3; var_dump($u);
$deep = ['a' => ['b' => ['c' => 'd']]]; var_dump(unserialize(serialize($deep)) === $deep);
// Typed properties
class T { public int $i = 0; public ?string $s = null; }
var_dump(unserialize('O:1:"T":2:{s:1:"i";i:5;s:1:"s";s:2:"ok";}'));
try { var_dump(unserialize('O:1:"T":1:{s:1:"i";s:3:"bad";}')); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
// readonly class and dynamic property errors
final class RO { public function __construct(public readonly int $v = 1) {} }
echo $s = serialize(new RO(5)), "\n"; var_dump(unserialize($s));
var_dump(unserialize('O:2:"RO":1:{s:1:"z";i:1;}'));
