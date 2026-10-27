<?php
// Round trips of nested structures with shared references
$a = ['x' => 1]; $b = ['p' => &$a, 'q' => &$a, 'r' => $a];
echo $s = serialize($b), "\n"; $u = unserialize($s); $u['p']['x'] = 5; var_dump($u);
$o1 = new stdClass; $o2 = new stdClass; $o1->peer = $o2; $o2->peer = $o1;
echo $s = serialize([$o1, $o2]), "\n"; $u = unserialize($s); var_dump($u[0]->peer === $u[1], $u[1]->peer === $u[0]);
// __serialize returning shared objects and references
class Node { public $children = []; public $parent; function __construct(public string $name = '') {} }
$root = new Node('root'); $c = new Node('c'); $c->parent = $root; $root->children[] = $c; $root->children[] = $c;
echo $s = serialize($root), "\n"; $u = unserialize($s); var_dump($u->children[0] === $u->children[1], $u->children[0]->parent === $u);
class W { public $n; function __serialize(): array { return ['self' => $this, 'n' => $this->n]; } function __unserialize(array $d): void { $this->n = $d['n']; var_dump($d['self'] === $this); } }
$w = new W; $w->n = 3; echo $s = serialize([$w, $w]), "\n"; var_dump(unserialize($s));
// Exceptions from magic methods
class Throws { function __wakeup() { throw new RuntimeException("no wakeup"); } }
try { unserialize(serialize(new Throws)); } catch (RuntimeException $e) { echo $e->getMessage(), "\n"; }
class ThrowsS { function __sleep() { throw new LogicException("no sleep"); } }
try { serialize(new ThrowsS); } catch (LogicException $e) { echo $e->getMessage(), "\n"; }
// Protected/private renamed keys map onto declarations
class V { public $pub = 1; protected $prot = 2; private $priv = 3; }
var_dump(unserialize('O:1:"V":3:{s:3:"pub";i:9;s:4:"prot";i:8;s:4:"priv";i:7;}'));
var_dump(unserialize("O:1:\"V\":2:{s:6:\"\0*\0pub\";i:6;s:7:\"\0V\0prot\";i:5;}"));
var_dump(unserialize("O:1:\"V\":1:{s:2:\"\0x\";i:6;}"));
// Deep nesting and depth limits
$d = 1; for ($i = 0; $i < 50; $i++) $d = [$d]; var_dump(unserialize(serialize($d)) === $d);
var_dump(unserialize(serialize($d), ['max_depth' => 10]));
var_dump(strlen(serialize(str_repeat('x', 1000))));
// Objects: DateTime keeps working through var_dump of unserialized plain objects
echo serialize([1.0, 0.30000000000000004, -1.5e-7, 123456789012345678]), "\n";
var_dump(unserialize('a:2:{i:0;i:1;i:0;i:2;}'));
var_dump(unserialize('a:1:{i:0;a:1:{i:0;R:2;}}'));
var_dump(unserialize('a:2:{i:0;i:1;i:1;R:1;}'));
var_dump(unserialize('a:2:{i:0;O:8:"stdClass":0:{}i:1;r:1;}'));
var_dump(unserialize('a:2:{i:0;s:1:"x";i:1;r:2;}'));
var_dump(unserialize('O:8:"stdClass":1:{s:1:"a";O:8:"stdClass":1:{s:1:"b";r:1;}}'));
var_dump(serialize(1.0E+25), serialize(-0.0), serialize(0.1 + 0.2));
abstract class Abs {} interface If1 {}
try { unserialize('O:3:"Abs":0:{}'); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { unserialize('O:3:"If1":0:{}'); } catch (Error $e) { echo $e->getMessage(), "\n"; }
enum E1 { case A; const B = 1; }
var_dump(unserialize('E:4:"E1:B";'));
var_dump(unserialize('E:4:"E1:A";') === E1::A);
