<?php
function attempt($f){try{var_dump($f());}catch(Throwable $e){echo get_class($e),': ',$e->getMessage(),"\n";}}
class Values {public int $a=1; public $b=2; protected $hidden=3;private $secret=4;public int $unset;public function change(){foreach($this as $key=>&$value){echo $key,' ';$value*=2;}echo "|\n";}}
$o=new Values;
foreach($o as $key=>&$value){echo $key,"\n";$value+=10;}unset($value);
var_dump($o->a,$o->b);$o->change();var_dump($o->a,$o->b);
foreach((object)['a'=>1,'b'=>2] as &$value){$value++;}var_dump($value);
$o=(object)['a'=>1,'b'=>2,'c'=>3];foreach($o as $key=>&$value){echo $key,' ';if($key==='a'){unset($o->b);$o->d=4;}$value*=10;}echo "|\n";var_dump($o);
class Locked {public readonly int $x;public function __construct(){$this->x=1;}}
attempt(function(){foreach(new Locked as &$value){}});
$o=new Values;foreach($o as $key=>&$value){if($key==='a'){attempt(function()use(&$value){$value=[];});break;}}var_dump($o->a);
function &numbers(&$a){foreach($a as &$v){yield $v;}unset($v);}
$a=[1,2,3];foreach(numbers($a) as $key=>&$value){echo $key,' ';$value*=10;}echo "|\n";unset($value);var_dump($a);
function &counter(){ $i=0;while($i<3){yield $i;$i++;} }
foreach(counter() as &$value){echo $value,"\n";$value++;}unset($value);
function &temporary(){yield 1;yield;}
foreach(temporary() as &$value){var_dump($value);}
$closure=function &(&$x){yield $x;};$x=5;foreach($closure($x) as &$value){$value=8;}unset($value);var_dump($x);
class Generators {public function &values(&$x){yield $x;}}
$x=10;foreach((new Generators)->values($x) as &$value){$value=11;}unset($value);var_dump($x);
class Aggregate implements IteratorAggregate {public $v=3;public function getIterator():Traversable {return $this->values();}private function &values(){yield $this->v;}}
$agg=new Aggregate;foreach($agg as &$value){$value=4;}unset($value);var_dump($agg->v);
function regular(){yield 1;}attempt(function(){foreach(regular() as &$v){}});
function inside(&$array){foreach($array as &$v){$v*=2;yield $v;}unset($v);}
$a=[1,2,3];var_dump(iterator_to_array(inside($a)),$a);
function inside_object($o){foreach($o as &$v){$v++;yield $v;}unset($v);}
$o=(object)['a'=>1,'b'=>2];var_dump(iterator_to_array(inside_object($o)),$o->a,$o->b);
