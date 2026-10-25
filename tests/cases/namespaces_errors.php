<?php
namespace A;
function f(int $x) { return $x; }
try { f("a"); } catch (\TypeError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
echo strlen([]);
