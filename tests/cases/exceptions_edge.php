<?php
$f = function () use (&$f) { return 1; };
var_dump($f);
print_r($f); echo "\n";
function thrower() { throw new Exception("x"); }
for ($i = 0; $i < 3; $i++) { try { thrower(); } catch (Exception $e) {} }
var_dump(new Exception("id reuse"));
$e1 = new Exception("a"); $e2 = new Exception("b", 0, $e1); unset($e1);
var_dump($e2->getPrevious()->getMessage());
try {
    try { throw new Exception("try"); }
    finally { throw new LogicException("from finally"); }
} catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), " prev=", var_export_ok($e), "\n"; }
function var_export_ok($e) { return $e->getPrevious() ? $e->getPrevious()->getMessage() : "none"; }
