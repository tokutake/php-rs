<?php
enum E { case A; }
$d = new DateTime('2024-01-02 03:04:05', new DateTimeZone('UTC'));
echo $s = serialize($d), "\n";
$u = unserialize($s); var_dump($u); echo $u->format('Y-m-d H:i:s e'), "\n";
echo serialize(new DateTimeZone('Europe/Paris')), "\n";
echo serialize(new DateInterval('P1D')), "\n";
class Par { private $x = 'px'; protected $y = 'py'; }
class Chi extends Par { public $z = 1; public int $t; function __sleep() { return ['x', 'y', 'z', 't']; } }
echo serialize(new Chi), "\n";
class Wk { public $inner; function __wakeup() { $this->inner = unserialize('a:1:{i:0;i:7;}'); } }
var_dump(unserialize(serialize(new Wk)));
var_dump(unserialize(serialize(E::A), ['allowed_classes' => false]));
$u = unserialize('O:1:"X":2:{s:1:"a";i:1;s:4:"' . "\0X\0b" . '";i:2;}'); var_dump($u); echo serialize($u), "\n";
