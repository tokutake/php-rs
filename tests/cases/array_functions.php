<?php
$a = [3, 1, 2];
echo count($a), " ", sizeof([]), "\n";
echo implode(", ", $a), "|", implode($a), "|", join("-", ['x' => 'a', 'b']), "\n";
var_dump(explode(",", "a,b,,c"), explode(",", "a,b,c,d", 2), explode(",", "a,b,c,d", -2), explode(",", "a,b", 0), explode(",", ""), explode(", ", "x, y"));
var_dump(array_keys(['a' => 1, 5 => 2, 'c' => 3]), array_values(['a' => 1, 'b' => 2]));
var_dump(in_array("2", [1, 2, 3]), in_array("2", [1, 2, 3], true), in_array("abc", [0]), in_array(null, [0]));
var_dump(array_key_exists('a', ['a' => null]), array_key_exists(1, ['1' => 'x']), array_key_exists('b', ['a' => 1]));
var_dump(range(1, 5), range(5, 1, 2), range('a', 'e', 2), range(0, 1, 0.25), range('A', 'z', 20));
var_dump(array_merge([1, 2], ['a' => 'x', 3], ['a' => 'y', 5 => 4]));
var_dump(array_reverse([1, 2, 'k' => 3]), array_reverse([1, 2, 3], true));
var_dump(array_sum([1, 2, 3.5, "4"]), array_sum([]));
$s = [5, 3, 10, 1]; sort($s); var_dump($s);
$s = ["banana", "apple", "cherry"]; rsort($s); var_dump($s);
$s = ["10", 9, "9a", 1.5]; sort($s); var_dump($s);
$st = []; echo array_push($st, 'a', 'b'), " "; echo array_pop($st), " ", count($st), "\n";
$st = [1, 2, 3]; array_pop($st); $st[] = 'x'; var_dump($st);
var_dump(is_array([]), is_array("a"), str_repeat("ab", 3));
