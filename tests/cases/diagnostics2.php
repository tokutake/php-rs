<?php
$s = ""; $s++; var_dump($s);
$s = ""; $s--; var_dump($s);
$s = "abc"; $s--; var_dump($s);
$b = true; $b++; $b--; var_dump($b);
var_dump((int)"1e19", (int)"9999999999999999999", (int)" 12", (int)"1.9e3");
var_dump(1e19 % 7);
var_dump(isset("abc"["1x"]), isset("abc"[1.7]), isset("abc"["1"]), isset("abc"[" 1"]));
var_dump("abc"[true]);
$a = [1]; var_dump($a[1.0]);
$str = "x"; $str[1.5] = "y"; var_dump($str);
echo "a" . [] . "\n";
var_dump("$a");
$r = "abc" + 1;
echo "not reached
";
