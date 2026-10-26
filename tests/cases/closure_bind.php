<?php
// Closure::bind(), bindTo() and call(), static closures
class P { private $secret = 's'; private static $ss = 'static secret'; public function m() { return 'm'; } public static function sm() { return static::class; }
  public function getC() { return function () { return $this->secret; }; } }
class Q extends P {}
function named() {}
$sc = static function () { return isset($this); };
var_dump(Closure::bind($sc, new P));
var_dump(Closure::bind(named(...), null, P::class));
var_dump(Closure::bind((new P)->m(...), new stdClass));
var_dump(Closure::bind((new P)->m(...), null));
var_dump(Closure::bind((new P)->getC(), null));
var_dump(Closure::bind(function () {}, null, 'stdClass'));
var_dump(Closure::bind(function () {}, null, 'Nope'));
var_dump(Closure::bind(P::sm(...), null, P::class) instanceof Closure);
$peek = function () { return $this->secret; };
try { $peek->bindTo(new P)(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
echo Closure::bind($peek, new P, P::class)(), "\n";
echo $peek->call(new Q), "\n";
$st = fn() => static::$ss;
echo Closure::bind($st, null, P::class)(), "\n";
try { Closure::bind(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $peek->bindTo(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $peek->call(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $peek->bindTo(5); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $peek->bindTo(null, []); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { Closure::bind(5, null); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $peek->call(5); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$who = function () { return [static::class, self::class, get_class($this)]; };
var_dump(Closure::bind($who, new Q, P::class)());
$b = $who->bindTo(new Q);
try { $b(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$counter = function () { return ++$this->n; };
$o = new stdClass; $o->n = 0;
$inc = Closure::bind($counter, $o);
$inc(); $inc();
var_dump($o->n);
$args = function (...$a) { return implode(',', $a) . ':' . $this->n; };
var_dump($args->call($o, 1, 2, 3));
$gen = function () { yield $this->n; };
foreach ($gen->call($o) as $v) var_dump($v);
class InMethod {
    public $v = 'iv';
    public function closures() {
        return [
            function () { return isset($this) ? $this->v : 'no this'; },
            static function () { return isset($this) ? $this->v : 'no this'; },
            fn () => isset($this) ? $this->v : 'no this',
            static fn () => isset($this) ? $this->v : 'no this',
        ];
    }
}
foreach ((new InMethod)->closures() as $f) {
    var_dump($f());
}
[$plain, $static] = (new InMethod)->closures();
var_dump(Closure::bind($plain, null, InMethod::class));
var_dump(Closure::bind($static, null, InMethod::class)());
$noThis = function () { return 1; };
var_dump(Closure::bind((new InMethod)->closures()[0]->bindTo(new InMethod), new InMethod)());
$named = Closure::bind(closure: $peek, newScope: P::class, newThis: new P);
echo $named(), "\n";
echo $peek->bindTo(newScope: 'P', newThis: new P)(), "\n";
echo $peek->call(newThis: new P), "\n";
