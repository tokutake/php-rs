<?php
foreach ([['5.2', '5.2.0'], ['1.0rc1', '1.0'], ['1.0', '1.0.0'], ['7.4', '8.0'], ['8.0.1', '8.0.0'], ['1.0-dev', '1.0alpha'], ['1.0a', '1.0b'], ['1.0pl1', '1.0'], ['1.0#1', '1.0.1'], ['', '1'], ['1..2', '1.2'], ['1.0RC1', '1.0rc1'], ['5.6.40-1+ubuntu', '5.6.40'], ['10', '9']] as [$a, $b]) {
    echo "$a vs $b: ", version_compare($a, $b), " ", var_export(version_compare($a, $b, '>='), true), " ", var_export(version_compare($a, $b, 'lt'), true), "\n";
}
try { version_compare('1', '2', 'bad'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(substr_compare("Hello", "ello", 1), substr_compare("Hello", "ELLO", 1, 4, true), substr_compare("abc", "b", -2, 1), substr_compare("abc", "bc", 1, 5));
try { substr_compare("abc", "b", 5); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$tok = strtok("a,b,,c;d", ",;"); while ($tok !== false) { echo "[$tok]"; $tok = strtok(",;"); } echo "\n";
var_dump(strtok("", " "), strtok("  x  y", " "), strtok(" "), strtok(" "));
print_r(array_change_key_case(['A' => 1, 'b' => 2, 5 => 3, 'C' => 4], CASE_UPPER));
print_r(array_change_key_case(['A' => 1, 'a' => 2]));
print_r(array_replace_recursive(['a' => [1, 2, 'x' => ['y' => 1]], 'b' => 2], ['a' => [9, 'x' => ['z' => 2]], 'c' => 3], ['b' => [1]]));
var_dump(array_first(['x' => 1, 'y' => 2]), array_last([1, 2, 3]), array_first([]), array_last([]));
var_dump(array_any([1, 2, 3], fn($v) => $v > 2), array_all([1, 2, 3], fn($v, $k) => $v > $k), array_find(['a' => 1, 'b' => 2], fn($v) => $v > 1), array_find_key(['a' => 1, 'b' => 2], fn($v) => $v > 1), array_find([1], fn($v) => false));
var_dump(utf8_encode("caf\xe9"), bin2hex(utf8_decode("café€")));
var_dump(is_string(gethostname()), sleep(0), usleep(1));
var_dump(array_key_exists(1, [1 => null]), array_product([2, "3", 4.0]), array_product([]));
var_dump(str_word_count("Hello fri3nd, you're looking good today!"));
var_dump(nl2br("a\nb\r\nc"), strip_tags("<p>Hello <b>World</b><br/>!</p>", "<b>"), addcslashes("foo[bar]", 'A..Z'), stripcslashes('a\tb\x41'));
var_dump(array_walk_recursive_test());
function array_walk_recursive_test() { $a = [1, [2, 3]]; $s = 0; array_walk_recursive($a, function ($v) use (&$s) { $s += $v; }); return $s; }
var_dump(ucwords("hello world-foo bar_baz"), ucwords("hello-world", "-"), lcfirst("ABC"), quotemeta("1+1=2?"), strpbrk("This is a test", "st"), substr_count("hello hello", "ll"), strrchr("a/b/c", "/"), stristr("HayStack", "st"), strncasecmp("Hello", "hELLx", 4), ltrim("  xx  "), str_word_count("a b c"));
var_dump(array_map(null, [1, 2], ['a', 'b']), range('a', 'e', 2), array_flip(['a', 'b']), natsort_test(), array_merge(...[[1], [2]]));
function natsort_test() { $a = ["img12.png", "img10.png", "img2.png", "img1.png"]; natsort($a); return $a; }
