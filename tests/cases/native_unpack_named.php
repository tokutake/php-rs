<?php
function attempt($f){try{var_dump($f());}catch(Throwable $e){echo get_class($e),': ',$e->getMessage(),"\n";}}
var_dump(str_repeat(...['string'=>'ab'],times:3));
var_dump(substr(...['string'=>'abcdef'],length:2,offset:1));
var_dump(str_pad(...['x',5],pad_type:STR_PAD_LEFT,pad_string:'-'));
attempt(fn()=>sprintf(...['%s:%d','x'],...['n'=>1]));
function logit($label,$value){echo $label,"\n";return $value;}
var_dump(substr(...logit('spread',['string'=>'abcdef']),length:logit('length',2),offset:logit('offset',1)));
attempt(fn()=>strlen(...['string'=>'a'],string:logit('duplicate',1)));
attempt(fn()=>strlen(...['string'=>'a'],bad:logit('unknown',1)));
attempt(fn()=>substr(...['offset'=>1],...[0=>'abc']));
$a=[[3,1,2]];sort(...$a,flags:SORT_NUMERIC);var_dump($a);
$b=[5,2];$args=[&$b];rsort(...$args,flags:SORT_REGULAR);var_dump($b);
$matches=[];$args=['/[a-z]+/','abc12',&$matches];var_dump(preg_match(...$args,flags:0),$matches);
var_dump(preg_match(...['/x/','x'],offset:0));
function values(){yield 'string'=>'abc';yield 'times'=>2;}
var_dump(str_repeat(...values()));
$d=DateTimeImmutable::createFromFormat(...['!Y-m-d','2024-01-02'],timezone:new DateTimeZone('UTC'));echo $d->format('c'),"\n";
var_dump(call_user_func(...['str_repeat'],times:2,string:'x'));
$call=str_repeat(...);var_dump($call(...['string'=>'x'],times:3));
function arrays(){yield [3,1];}
sort(...arrays());
function by_value(int $v){$v=9;return $v;}
$v=1;$pack=[&$v];var_dump(by_value(...$pack),$v);
function by_reference(&$v){$v++;}
$pack=[1];by_reference(...$pack);var_dump($pack);
readonly class Arguments {public function __construct(public array $args){}}
$readonly=new Arguments(['string'=>'x','times'=>2]);var_dump(str_repeat(...$readonly->args));
$readonly=new Arguments([3]);var_dump(by_value(...$readonly->args));
