<?php
$calls = [
    fn() => str_pad("x", 5, ""),
    fn() => str_pad("x", 5, " ", 7),
    fn() => str_repeat("x", -1),
    fn() => str_split("abc", 0),
    fn() => array_chunk([1], 0),
    fn() => array_combine([1, 2], [1]),
    fn() => array_fill(0, -1, 0),
    fn() => max([]),
    fn() => max(),
    fn() => max(1),
    fn() => round(1.5, 0, 99),
    fn() => strpos("abc", "a", 10),
    fn() => strrpos("abc", "a", -10),
    fn() => substr_count("abc", ""),
    fn() => intval("12", 1),
    fn() => base_convert("1", 1, 10),
    fn() => log(1, 0),
    fn() => sprintf("%z", 1),
    fn() => sprintf("%"),
    fn() => sprintf("%2\$s", 1),
    fn() => str_replace("a", ["b"], "abc"),
    fn() => strncmp("a", "b", -1),
    fn() => array_slice("x", 1),
    fn() => strlen([]),
    fn() => abs("abc"),
    fn() => wordwrap("abc def", 0, "\n", true),
    fn() => chunk_split("abc", 0),
    fn() => strpbrk("abc", ""),
    fn() => array_walk($undefined_var, 'strtoupper'),
];
foreach ($calls as $i => $f) {
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo $i, ": ", get_class($e), ": ", $e->getMessage(), "\n";
    }
}
var_dump(hex2bin("abc"), hex2bin("zz"), bindec("12"), array_flip([[1]]), array_count_values([1.5]), compact('nope'));
