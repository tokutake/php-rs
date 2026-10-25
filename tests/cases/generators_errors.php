<?php
function g1() {
    try {
        echo "try\n";
        yield 1;
        yield 2;
    } catch (Exception $e) {
        echo "caught in gen: ", $e->getMessage(), "\n";
        yield 99;
    } finally {
        echo "finally\n";
    }
    yield 3;
}
$g = g1();
echo $g->current(), "\n";
echo $g->throw(new Exception("boom")), "\n";
$g->next();
echo $g->current(), "\n";
$g->next();
var_dump($g->valid());

echo "-- uncaught in generator\n";
function g2() { yield 1; throw new RuntimeException("inside"); }
$g = g2();
try {
    foreach ($g as $v) echo "v=$v\n";
} catch (RuntimeException $e) {
    echo "caught outside: ", $e->getMessage(), "\n";
}
var_dump($g->valid());
try { $g->getReturn(); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }

echo "-- throw into finished / unstarted\n";
function g3() { echo "g3 start\n"; try { yield 1; } catch (Exception $e) { echo "g3 caught\n"; } }
$g = g3();
try { $g->throw(new LogicException("early")); } catch (LogicException $e) { echo "early propagated\n"; }
var_dump($g->valid());
$g = g3();
foreach ($g as $_) {}
try { $g->throw(new LogicException("late")); } catch (LogicException $e) { echo "late propagated\n"; }

echo "-- finally on destroy\n";
function g4() {
    try {
        yield 1;
        yield 2;
    } finally {
        echo "cleanup\n";
    }
}
$g = g4();
$g->current();
unset($g);
echo "after unset\n";
foreach (g4() as $v) { echo "v=$v\n"; break; }
echo "after break\n";
$g = g4();
unset($g);
echo "unstarted dropped\n";

echo "-- errors\n";
function g5() { yield 1; yield 2; }
$g = g5();
foreach ($g as $v) {}
try { foreach ($g as $v) {} } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$g = g5(); $g->current(); $g->next();
try { $g->rewind(); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$g = g5(); $g->rewind(); $g->rewind(); echo "rewind ok\n";
try { new Generator; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { clone g5(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
function g6() { global $self; yield $self->current(); }
$self = g6();
try { $self->current(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$g = g5();
try { $g->getReturn(); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
function g7() { return 5; yield; }
$g = g7();
var_dump($g->valid(), $g->getReturn());
var_dump(g5());
function g8(int $x) { yield $x; }
try { g8("abc"); } catch (TypeError $e) { echo "TypeError at call: ", $e->getMessage(), "\n"; }
function g9() { yield 1; return; }
$g = g9(); foreach ($g as $v); var_dump($g->getReturn());
echo "-- trace\n";
function g10() { yield 1; throw new Exception("t"); }
function run10() { foreach (g10() as $v) {} }
try { run10(); } catch (Exception $e) { echo $e->getTraceAsString(), "\n"; }
$g = g10(); $g->current();
try { $g->next(); } catch (Exception $e) { echo $e->getTraceAsString(), "\n"; }
