<?php
function attempt($f){try{var_dump($f());}catch(Throwable $e){echo get_class($e),': ',$e->getMessage(),"\n";}}
class A {public static $x=1;public static $y=2;public static int $typed=3;protected static $hidden=4;public static function &hidden(){return self::$hidden;} public static function test(){ $r=&self::$hidden;$r=7;return self::$hidden;}}
class B extends A {}
$r=&A::$x;$r=10;var_dump(A::$x,B::$x);B::$x=11;var_dump($r);
$v=20;A::$y=&$v;$v=21;var_dump(A::$y);A::$y=22;var_dump($v);
A::$x=&A::$y;A::$x=30;var_dump(A::$y,$v,$r);
$values=[1,2];A::$x=&$values[0];A::$x++;var_dump($values);
$t=&A::$typed;$t='12';var_dump($t,A::$typed);
attempt(function()use(&$t){$t=[];});var_dump($t,A::$typed);
var_dump(A::test());attempt(function(){$r=&A::$hidden;});
A::$x=[1,2];$arr=&A::$x;$arr[]=3;var_dump(A::$x);
class Box {public int $x=5;}
$box=new Box;$x=&$box->x;$x='6';var_dump($x,$box->x);
attempt(function()use(&$x){$x=[];});var_dump($box->x);
