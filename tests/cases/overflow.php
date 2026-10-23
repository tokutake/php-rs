<?php
// Edge cases that tend to be undefined behavior in C
var_dump(PHP_INT_MAX + 1, PHP_INT_MIN - 1, PHP_INT_MAX * 2, -PHP_INT_MIN);
var_dump(PHP_INT_MIN % -1, PHP_INT_MIN / -1, intdiv(7, 2), 2 ** 63, 2 ** 62, (-2) ** 63);
var_dump(1 << 64, 1 << 63, -1 >> 63, -8 >> 1, 1 << 3);
$i = PHP_INT_MAX; $i++; var_dump($i);
$j = PHP_INT_MIN; $j--; var_dump($j);
var_dump(9223372036854775807, 9223372036854775808, 0x7FFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF);
$n = null; $n++; var_dump($n);
var_dump("abc" & "  a", "a" | "bcd", ~5, 6 ^ 3);
