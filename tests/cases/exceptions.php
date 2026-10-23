<?php
function t(callable $c) {
    try {
        $c();
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), " (line ", $e->getLine(), ")\n";
    }
}

// Basic flow
try {
    echo "before\n";
    throw new InvalidArgumentException("bad input", 7);
    echo "not reached\n";
} catch (RuntimeException $e) {
    echo "wrong catch\n";
} catch (LogicException $e) {
    echo "caught ", get_class($e), " code=", $e->getCode(), "\n";
} finally {
    echo "finally\n";
}

// Multi-catch, catch without variable, rethrow
try {
    try {
        throw new DomainException("inner");
    } catch (TypeError | DomainException $e) {
        echo "first: ", $e->getMessage(), "\n";
        throw $e;
    }
} catch (Exception) {
    echo "rethrown and caught without a variable\n";
}

// finally runs on return, and its return wins
function f1() { try { return "try"; } finally { echo "f1 finally\n"; } }
function f2() { try { return "try"; } finally { return "finally"; } }
function f3() { try { throw new Exception("lost"); } finally { return "finally swallows"; } }
var_dump(f1(), f2(), f3());

// finally inside loops with break/continue
foreach ([1, 2, 3] as $i) {
    try {
        if ($i == 2) continue;
        if ($i == 3) break;
        echo "body $i\n";
    } finally {
        echo "finally $i\n";
    }
}

// Exceptions cross function boundaries with their trace
function level3($x) { throw new RuntimeException("deep $x"); }
function level2($x) { level3($x . "!"); }
try { level2("arg"); } catch (RuntimeException $e) {
    echo $e->getTraceAsString(), "\n";
    print_r($e->getTrace());
}

// Internal errors are catchable
t(fn() => 1 % 0);
t(fn() => intdiv(PHP_INT_MIN, -1));
t(fn() => [] + 1);
t(fn() => "abc" * 2);
t(fn() => undefined_function());
t(fn() => strlen());
t(fn() => strlen([]));
t(fn() => intdiv("5 apples", 1));
t(fn() => str_repeat("x", -1));
t(fn() => new Closure());
t(fn() => new Throwable());
t(fn() => new NoSuchClass());
t(function () { throw new Exception(); });
t(function () { throw 5; });
t(function () { $x = null; $x->method(); });
t(function () { $e = new Exception("m"); return $e->message; });
t(function () { $e = new Exception("m"); $e->nope(); });

// Exception objects
$prev = new LogicException("first", 1);
$e = new RuntimeException("second", 2, $prev);
var_dump($e->getPrevious() === $prev, $e->getPrevious()->getMessage(), $e instanceof Exception, $e instanceof Throwable, $e instanceof Error, $prev instanceof RuntimeException);
echo $e, "\n";
var_dump(new Exception("m", 3));
$err = new TypeError("typed");
echo get_class($err), " ", $err instanceof Error ? "is Error" : "", "\n";

// throw as an expression
$value = null;
try { $x = $value ?? throw new UnexpectedValueException("no value"); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
$fn = fn($v) => $v ?: throw new LengthException("empty");
try { $fn(""); } catch (LengthException $e) { echo get_class($e), "\n"; }

// Uncaught, with a previous exception
function fail() { throw new RuntimeException("outer", 0, new LogicException("cause")); }
fail();
