<?php
// Output buffering: the ob_* functions, handlers and their phases

echo "-- without a buffer\n";
var_dump(ob_get_level(), ob_get_contents(), ob_get_length(), ob_get_clean(), ob_get_flush());
var_dump(ob_end_clean(), ob_end_flush(), ob_clean(), ob_flush());

echo "-- capturing and nesting\n";
ob_start();
echo "hello";
$x = ob_get_clean();
var_dump($x);
ob_start();
echo "a";
ob_start();
echo "b";
var_dump(ob_get_level());
$inner = ob_get_contents();
ob_end_flush();
echo "c";
$len = ob_get_length();
$all = ob_get_clean();
var_dump($inner, $all, $len);

echo "-- handlers\n";
ob_start(function ($buf, $phase) { return strtoupper($buf) . "[$phase]"; });
echo "x";
ob_flush();
echo "y";
ob_clean();
echo "z";
ob_end_flush();
echo "\n";
ob_start(null, 4);
echo "12";
echo "345";
echo "6";
$chunked = ob_get_contents();
ob_end_clean();
var_dump($chunked);
function wrap($b) { return "<$b>"; }
class Upper { public function __invoke($b) { return strtoupper($b); } public function m($b) { return "m($b)"; } }
ob_start('wrap');
ob_start(new Upper);
ob_start([new Upper, 'm']);
ob_start(fn ($b) => $b);
print_r(ob_list_handlers());
ob_end_flush();
ob_end_flush();
ob_end_flush();
ob_end_flush();
echo "\n";

echo "-- printing functions and diagnostics go to the buffer\n";
ob_start();
printf("%s-%d ", "p", 5);
print "printed ";
var_dump(true);
echo $undefined;
$captured = ob_get_clean();
var_dump($captured);
ob_start();
fwrite(STDOUT, "STDOUT bypasses buffers\n");
$out = fopen('php://output', 'w');
fwrite($out, "php://output does not");
$captured = ob_get_clean();
var_dump($captured);

echo "-- status and flags\n";
ob_start();
echo "abc";
$a = ob_get_status();
ob_start(function ($b) { return $b; }, 10);
echo "x";
$b = ob_get_status(true);
ob_end_clean();
ob_end_clean();
var_export([$a, $b]);
echo "\n";
ob_start(null, 0, 0);
var_dump(ob_clean(), ob_flush(), ob_end_flush(), ob_end_clean(), ob_get_clean(), ob_get_flush());
$stuck = ob_get_contents();
ob_start(null, 0, PHP_OUTPUT_HANDLER_CLEANABLE);
var_dump(ob_clean());
ob_start('nope');
ob_start([1, 2]);
var_dump(ob_get_level());

echo "-- handler exceptions\n";
function h($buf, $phase) {
    if (str_contains($buf, "boom")) {
        throw new RuntimeException("handler: $phase");
    }
    return "<$buf>";
}
ob_start('h');
echo "boom";
try {
    ob_end_flush();
} catch (RuntimeException $e) {
    echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n";
}
ob_start(function ($b, $p) { return "[$p:" . strlen($b) . "]"; });
echo "left open";
