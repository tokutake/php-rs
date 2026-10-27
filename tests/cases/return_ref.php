<?php
function val() { return [1]; }
function &ref() { static $s = [1]; return $s; }
class O { public $p = 5; function make() { return new O; } function &rp() { return $this->p; } static function sm() { return 3; } }
$a = &val(); $a[] = 2; var_dump($a);
$b = &ref(); $b[] = 2; $c = &ref(); var_dump($c);
$o = new O;
$e = &$o->make(); var_dump($e);
$f = &$o->rp(); $f = 9; var_dump($o->p);
$g = &O::sm(); var_dump($g);
$arr = []; $arr['k'] = &val(); var_dump($arr);
$o->q = &val();
