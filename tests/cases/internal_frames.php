<?php
function f($s) {
    return strlen($s);
}
function show(Throwable $e) {
    echo get_class($e), ": ", $e->getMessage(), " @", $e->getLine(), "\n", $e->getTraceAsString(), "\n";
}
try { f([1]); } catch (TypeError $e) { show($e); }
try { str_repeat("x", -1); } catch (ValueError $e) { show($e); }
try { intdiv(1, 0); } catch (DivisionByZeroError $e) { show($e); }
try { new Exception([]); } catch (TypeError $e) { show($e); }
enum E: int { case A = 1; }
try { E::from(2); } catch (ValueError $e) { show($e); }
try { E::from("x"); } catch (TypeError $e) { show($e); }
function thrower($x) { throw new LogicException("in callback"); }
try { array_map("thrower", [1]); } catch (LogicException $e) { show($e); }
try { usort($arr, fn () => 0); } catch (TypeError $e) { show($e); }
$e = new Exception("x");
try { $e->getMessage(1); } catch (ArgumentCountError $e2) { show($e2); }
echo strlen(...[1, 2]);
