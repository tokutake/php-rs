<?php
// Messages and checks that changed after PHP 8.3 (the .out is edited by hand
// to PHP 8.5's behaviour)
foreach (["abc", "\\abc\\", "\0abc\0", "/a/e", "/a/\0", "/a/r"] as $p) {
    $r = preg_match($p, "abc");
    echo json_encode($p), " => ", var_export($r, true), "\n";
}
try {
    preg_match('/a/', 'a', $m, 0, PHP_INT_MIN);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
