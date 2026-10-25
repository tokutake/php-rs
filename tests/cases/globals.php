<?php
$a = 1;
var_dump($GLOBALS["nope"] ?? "dflt");
echo $GLOBALS["nope"];
function f() { $GLOBALS["n"]["k"] = 1; $GLOBALS["a"]++; $r = &$GLOBALS["a"]; $r = 50; return $GLOBALS['a']; }
var_dump(f(), $n, $a);
foreach ($GLOBALS as $k => $v) { if ($k == "a") echo "found a=$v\n"; }
$g = $GLOBALS; $g["a"] = 99; var_dump($a);
unset($GLOBALS["n"]); var_dump(isset($n), isset($GLOBALS['a']), array_key_exists('g', $GLOBALS));
