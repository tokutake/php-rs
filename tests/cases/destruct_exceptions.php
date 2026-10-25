<?php
class D { public function __construct(public $n) {} public function __destruct() { echo "dtor {$this->n}\n"; } }
function g() {
  try { $b = new D(2); throw new Exception("x"); }
  finally { echo "finally\n"; }
}
try { g(); } catch (Exception $e) { echo "caught\n"; }
class E { public function __destruct() { throw new Exception("dtor ex"); } }
function h() { $e = new E; throw new Exception("orig"); }
try { h(); } catch (Exception $e) { echo get_class($e), " ", $e->getMessage(), " prev=", $e->getPrevious()?->getMessage(), "\n"; }
$o = new D(5);
$o2 = $o;
$o->__destruct();
unset($o, $o2);
echo "x\n";
class R { public function __destruct() { global $saved; $saved = $this; echo "resurrect\n"; } }
$r = new R; unset($r); echo "after\n"; var_dump($saved); $saved = null; echo "end\n";
