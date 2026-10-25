<?php
class D { public function __construct(public $n) {} public function __destruct() { echo "dtor {$this->n}\n"; } }
class T { public function __destruct() { echo "T\n"; throw new Exception("t"); } }
$a = new D(1);
$t = new T;
$b = new D(2);
