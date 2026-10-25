<?php
echo "in a: ", basename(__FILE__), " ", basename(__DIR__), " x=$x\n";
$fromA = 'set by a';
if (!function_exists('a_func')) {
    function a_func($n) { return $n * 100; }
    function a_throw() { throw new Exception("from a"); }
    function a_warn() { return $undefined_in_a; }
}
return 'a-result';
