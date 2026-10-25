<?php
// First-class callables: namespaces, scopes and calls through built-ins
namespace App {
    function hello($n) { return "hello $n"; }
    $h = hello(...);
    echo $h('ns'), "\n";
    $l = strlen(...);
    echo $l('four'), "\n";
}
namespace {
class P {
    public static function make() { return static::class; }
    public function who() { return 'P'; }
    public static function lsb() { return static::make(...); }
    private function secret($x) { return "secret $x"; }
    public function expose() { return Closure::fromCallable([$this, 'secret']); }
}
class Q extends P {
    public function who() { return 'Q'; }
    public function parentWho() { return parent::who(...); }
}
$q = new Q;
var_dump(Q::lsb()(), $q->parentWho()(), $q->expose()('x'));
$c = function () { return 1; };
var_dump($c(...) === $c);
$w = fn(&$x) => $x++;
function inc(&$x) { $x++; }
$i = inc(...);
$arr = [1, 2, 3];
array_walk($arr, $i);
var_dump($arr);
$v = 1;
$i($v);
var_dump($v);
usort($arr, (fn($a, $b) => $b <=> $a)(...));
var_dump($arr);
var_dump(is_callable(strlen(...)), call_user_func(str_repeat(...), 'ab', 2), call_user_func_array(str_repeat(...), ['times' => 3, 'string' => 'c']));
function t() { $f = strlen(...); return $f([]); }
try { t(); } catch (TypeError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
function u($a) { throw new Exception('u'); }
try { $f = u(...); array_map($f, [1]); } catch (Exception $e) { echo $e->getTraceAsString(), "\n"; }
try { $f = u(...); $f->__invoke(5); } catch (Exception $e) { echo $e->getTraceAsString(), "\n"; }
$o = null;
try { $o->m(...); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { (new P)->secret(...); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { P::who(...); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $x = 5; $x(...); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $x = new P; $x(...); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { Closure::fromCallable(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { Closure::fromCallable([1, 2]); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$gen = function () { yield 1; yield 2; };
foreach ($gen(...)() as $g) echo $g;
echo "\n";
$m = max(...);
var_dump($m(1, 5, 3), $m(...[4, 9]));
}
