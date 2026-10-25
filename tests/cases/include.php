<?php
declare(strict_types=1);
echo "main ", basename(__FILE__), " ", basename(__DIR__), "\n";
$x = 10;
$r = include 'inc/a.php';
var_dump($r, $fromA, a_func(2));
var_dump(include 'inc/a.php');
var_dump(include_once 'inc/a.php');
var_dump(require_once __DIR__ . '/inc/b.php', require_once 'inc/b.php');
var_dump(B::hello());
function f() { $local = 1; include 'inc/scope.php'; return $inScope + $local; }
var_dump(f());
var_dump((include 'inc/missing.php'));
echo "after missing\n";
var_dump(@include 'inc/missing.php');
try { a_throw(); } catch (Exception $e) { echo basename($e->getFile()), ":", $e->getLine(), "\n", str_replace(__DIR__, '', $e->getTraceAsString()), "\n"; }
var_dump(a_warn());
var_dump($GLOBALS['x'], isset($GLOBALS['nope']));
$GLOBALS['viaGlobals'] = 5;
var_dump($viaGlobals);
function g() { return $GLOBALS['x'] * 2; }
var_dump(g());
function setg() { $GLOBALS['made'] = 'yes'; }
setg(); var_dump($made);
unset($GLOBALS['made']); var_dump(isset($made));
var_dump(array_key_exists('x', $GLOBALS), count($GLOBALS) > 3);
// strict_types belongs to each file: the included one coerces
var_dump(include 'inc/coerce.php');
try { strlen(5); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
// Errors in included code: traces show the include like a call
function inc_mode($m) { $mode = $m; include __DIR__ . '/inc/t.php'; }
foreach ([1, 2, 4] as $m) {
    // (only the start of the trace: the include's argument is the absolute path)
    try { inc_mode($m); } catch (Throwable $e) { echo basename($e->getFile()), ":", $e->getLine(), "\n", str_replace(__DIR__, '', explode("\n", $e->getTraceAsString())[0]), "\n"; }
}
try { include 'inc/broken.php'; } catch (ParseError $e) { echo get_class($e), ": ", $e->getMessage(), " in ", basename($e->getFile()), ":", $e->getLine(), "\n"; }
var_dump(basename('/a/b/c.txt'), basename('/a/b/c.txt', '.txt'), basename('dir/'), dirname('/a/b/c.txt'), dirname('/a/b/c', 2), dirname('file'), dirname('/'), pathinfo('/x/y/archive.tar.gz'), pathinfo('/x/y/name', PATHINFO_EXTENSION));
var_dump(file_exists(__FILE__), is_file(__DIR__), is_dir(__DIR__), basename(realpath(__DIR__ . '/inc/../include.php')), strlen(file_get_contents(__DIR__ . '/inc/b.php')) > 0);
require 'inc/missing2.php';
echo "not reached\n";
