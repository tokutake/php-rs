<?php
$values = [0, 1, -5, 1.5, -0.0, NAN, INF, "12", "1.5", " 12", "12 ", "1e3", "0x1A", "abc", "", null, true, false, [], [1], new stdClass];
foreach ($values as $v) {
    echo str_pad(gettype($v), 8), " int:", (int)is_int($v), " float:", (int)is_float($v), " string:", (int)is_string($v),
        " bool:", (int)is_bool($v), " null:", (int)is_null($v), " numeric:", (int)is_numeric($v),
        " scalar:", (int)is_scalar($v), " countable:", (int)is_countable($v), " debug:", get_debug_type($v), "\n";
}
var_dump(is_integer(5), is_long(5), is_double(5.0));
echo "-- intval\n";
foreach (["42", "042", "0x1A", "0b11", "1e3", "12abc", " 7", "abc", 3.99, -3.99, true, null, [], [0]] as $v) {
    echo var_export($v, true), " => ", intval($v), "\n";
}
var_dump(intval("42", 8), intval("0x1A", 16), intval("0x1A", 0), intval("012", 0), intval("0b101", 0), intval("z", 36), intval("ff", 16));
var_dump(floatval("1.5e3abc"), floatval("abc"), floatval(" 2.5"), doubleval("3"), boolval("0"), boolval([]), boolval("0.0"), strval(1.0), strval(0.1 + 0.2), strval(true), strval(null));
echo "-- var_export\n";
var_export(1); echo "\n";
var_export(-1.5); echo "\n";
var_export(1.0); echo "\n";
var_export(0.1 + 0.2); echo "\n";
var_export(1e100); echo "\n";
var_export(-0.0); echo "\n";
var_export("it's a \\ test"); echo "\n";
var_export("nul\0byte"); echo "\n";
var_export(null); echo "\n";
var_export(false); echo "\n";
var_export([]); echo "\n";
var_export([1, 'a' => [true, null], 'b' => ['x' => 'y']]); echo "\n";
class P { public $a = 1; protected $b = [2]; private $c = 'three'; }
var_export(new P); echo "\n";
$o = new stdClass; $o->x = 1; $o->list = [1, 2];
var_export($o); echo "\n";
enum Suit: string { case Hearts = 'H'; }
var_export(Suit::Hearts); echo "\n";
var_export([Suit::Hearts]); echo "\n";
$s = var_export([1, 2], true);
echo strlen($s), "\n";
var_export(INF); echo "\n";
var_export(NAN); echo "\n";
var_export(PHP_INT_MIN); echo "\n";
