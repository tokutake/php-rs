<?php
class D { function __destruct() { echo "dtor\n"; } }
$d = new D;
function gen() { try { yield 1; } finally { echo "gen finally\n"; } }
$g = gen(); $g->current();
function f() { try { echo "in f\n"; exit("bye\n"); } finally { echo "finally (not run)\n"; } }
function g2() { try { yield 1; exit(); } finally { echo "running generator's finally (not run)\n"; } }
if ($argc ?? 0) {}
f();
echo "not reached\n";
