<?php
$r = "5 apples" + 1;
$r = " 5" + 1;
$r = "5 " + 1;
$r = 7 % 2.5;
$r = 1 << 1.5;
$r = (int)1e19;
$r = (int)"1e19";
$a = []; $a[1.5] = 1; $a[null] = 2; $a[true] = 3;
$r = $a[2.5] ?? 0;
$s = "abc"; $s++;
$n = null; $n--;
$a['x'] .= 'y';
$u .= 'z';
$r = $a[9][1];
$r = null[0];
$r = "abc"[10];
$five = 5; $r = $five[0];
echo [], "\n";
foreach (5 as $v) {}
$r = "abc"["1x"];
$r = "abc"[1.7];
$r = intdiv(1.5, 1);
$r = "1.5" | 1;
$r = -"abc";
$r = "abc" == 0;
