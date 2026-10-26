<?php
interface Label { public function label(): string; }
class Base {protected $v=4;public function base(){return $this->v;} }
trait Extra {public function extra(){return 'trait';}}
$a=new class {public $value=3;public function get(){return $this->value;}};
var_dump($a->get(),get_class($a));
$b=new class(7) extends Base implements Label {use Extra;public function __construct(private int $n){}public function label():string{return 'n'.$this->n;}public function plus(){return parent::base()+$this->n;}public function name(){return [__CLASS__,__METHOD__,__FUNCTION__];}};
var_dump($b instanceof Base,$b instanceof Label,$b->label(),$b->extra(),$b->plus(),$b->name());
function make($n){return new class($n){public static $count=0;public function __construct(public $n){self::$count++;}public function count(){return static::$count;}};}
$x=make(1);$y=make(2);var_dump(get_class($x)===get_class($y),$x!==$y,$x->n,$y->n,$y->count());
class Outer {private $secret=10;public function value(){return $this->secret;}public function make(){return new class($this){public function __construct(public Outer $outer){}public function get(){return $this->outer->value();}};}}
var_dump((new Outer)->make()->get());
$n=42;$f=fn()=>new class($n){public function __construct(public $n){}};var_dump($f()->n);
$nested=new class {public function inner(){return new class {public function get(){return 8;}};}};
var_dump($nested->inner()->get(),get_class($nested),get_class($nested->inner()));
$only=new class implements Label {public function label():string{return 'label';}};
var_dump(get_class($only),$only->label());
$copy=clone $a;$copy->value=9;var_dump($a->value,$copy->value);
$ro=new readonly class(5){public function __construct(public int $n){}};var_dump($ro->n);
function gen(){return new class(yield 'first'){public function __construct(public $n){}public function stream(){yield 2;yield 3;}};}
$g=gen();var_dump($g->current());$g->send(12);$result=$g->getReturn();var_dump($result->n,iterator_to_array($result->stream()));
var_dump(get_class((new class {public function create(){return new self;}})->create()));
