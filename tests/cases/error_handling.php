<?php
// error_reporting filters what is shown
var_dump(error_reporting());
$old = error_reporting(E_ALL & ~E_WARNING);
var_dump($old);
echo $undefined1;
error_reporting(E_ALL);
echo $undefined2;
var_dump(error_get_last());
error_clear_last();
var_dump(error_get_last());
@$x = $undefined3;
var_dump(error_get_last()['message']);

// set_error_handler sees warnings, notices and user errors
function handler($no, $str, $file, $line) {
    echo "handler($no): $str at line $line\n";
    return true;
}
var_dump(set_error_handler('handler'));
echo $undefined4, "after\n";
trigger_error("custom notice");
trigger_error("custom warning", E_USER_WARNING);
user_error("custom deprecated", E_USER_DEPRECATED);
$arr = []; $arr['x'];
// A handler returning false lets PHP report the error
set_error_handler(function ($no, $str) { echo "declined: $str\n"; return false; });
echo $undefined5;
restore_error_handler();
echo $undefined6;
restore_error_handler();
echo $undefined7;
// Handlers can be limited to some levels
set_error_handler(fn($no, $str) => print("only warnings: $str\n"), E_WARNING);
trigger_error("a notice");
echo $undefined8;
restore_error_handler();
// Converting errors to exceptions
set_error_handler(function ($no, $str, $file, $line) { throw new ErrorException($str, 0, $no, $file, $line); });
try {
    echo $undefined9;
    echo "not reached\n";
} catch (ErrorException $e) {
    echo "caught: ", $e->getMessage(), " severity ", $e->getSeverity(), "\n";
}
restore_error_handler();
trigger_error("plain user warning", E_USER_WARNING);
try { trigger_error("bad", 12345); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }

// assert

try { assert(false, "explained"); } catch (AssertionError $e) { echo $e->getMessage(), "\n"; }
var_dump(assert(true));

// ini settings
var_dump(ini_get('memory_limit'), ini_get('precision'), ini_get('nonexistent.setting'), ini_get('display_errors'));
var_dump(ini_set('memory_limit', '256M'), ini_get('memory_limit'), ini_set('nonexistent.setting', 'x'));
ini_restore('memory_limit'); var_dump(ini_get('memory_limit'));
var_dump(set_time_limit(30), ignore_user_abort(true));

// func_get_arg and debug_backtrace
function args() { return [func_get_arg(1), func_num_args()]; }
var_dump(args('a', 'b', 'c'));
class K { function m($x) { return debug_backtrace(); } static function s() { return (new K)->m(5); } }
function outer() { return K::s(); }
$bt = outer();
foreach ($bt as $f) { echo $f['function'], ' ', $f['class'] ?? '-', ' ', $f['type'] ?? '-', ' ', $f['line'] ?? '-', ' ', isset($f['object']) ? get_class($f['object']) : '-', ' ', json_encode($f['args'] ?? null), "\n"; }
$bt = (new K)->m(1); var_dump(count($bt), array_keys($bt[0]));
function nargs() { return debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS, 1); }
var_dump(array_keys(nargs()[0]));

// headers in the CLI
var_dump(headers_sent(), headers_list());
header('X-Test: 1');
var_dump(http_response_code());

// Shutdown functions and the exception handler
register_shutdown_function(function ($a) { echo "shutdown 1 $a\n"; }, 'arg');
register_shutdown_function(function () { echo "shutdown 2\n"; register_shutdown_function(fn() => print("nested shutdown\n")); });
class D { function __destruct() { echo "destructed\n"; } }
$d = new D;
set_exception_handler(function ($e) { echo "exception handler: ", $e->getMessage(), "\n"; });
throw new RuntimeException("uncaught");
