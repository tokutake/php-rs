<?php
$a = ['x' => 1, 'y' => 2, 'z' => 3];
var_dump(current($a), key($a), next($a), key($a), next($a), next($a), key($a), current($a), prev($a), reset($a), end($a), key($a), prev($a), prev($a), prev($a), current($a));
$b = $a; end($b); var_dump(key($a), key($b));
$c = [1, 2, 3]; next($c); unset($c[1]); var_dump(current($c), key($c));
$d = []; var_dump(current($d), key($d), reset($d), end($d));
$e = [5]; next($e); $e[] = 6; var_dump(current($e));
function walk(array $arr) { while (($v = current($arr)) !== false) { echo key($arr), "=", $v, " "; next($arr); } echo "\n"; }
walk(['a' => 1, 'b' => 2]);
$f = [1, 2]; foreach ($f as $v) {} var_dump(current($f));
$g = [1, 2, 3]; next($g); array_pop($g); var_dump(current($g)); $h = [1, 2]; end($h); $h[] = 3; var_dump(current($h));
$r = ['k' => &$x]; $x = 'refd'; var_dump(current($r), pos($r), reset($r));
var_dump(function_exists('asort'), function_exists('reset'), extension_loaded('json'), extension_loaded('nope'));
try { reset($str); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$big = range(1, 20); for ($i = 0; $i < 15; $i++) unset($big[$i]); var_dump(current($big), key($big));
$big2 = range(1, 20); end($big2); for ($i = 0; $i < 15; $i++) unset($big2[$i]); var_dump(current($big2), key($big2));
$a=[1,2,3]; end($a); array_shift($a); var_dump(current($a));
$a=[1,2,3]; end($a); array_unshift($a, 0); var_dump(current($a));
$a=[1,2,3]; end($a); array_splice($a, 0, 1); var_dump(current($a));
$a=[3,1,2]; end($a); sort($a); var_dump(current($a));
$a=[3,1,2]; end($a); asort($a); var_dump(current($a));
$a=[3,1,2]; end($a); array_push($a, 9); var_dump(current($a));
$a=[3,1,2]; end($a); $a[] = 5; var_dump(current($a)); $b = $a; var_dump(current($b));
$a=[3,1,2]; end($a); shuffle($a); var_dump(key($a));
$a=[3,1,2]; end($a); usort($a, fn($x,$y)=>$x<=>$y); var_dump(current($a));
