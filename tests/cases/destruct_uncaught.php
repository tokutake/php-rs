<?php
class D { public function __construct(public $n) {} public function __destruct() { echo "dtor {$this->n}\n"; } }
function f() { $x = new D(1); g(); }
function g() { throw new Error("e"); }
$y = new D(2);
f();
