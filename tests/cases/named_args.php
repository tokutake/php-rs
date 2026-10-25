<?php
function greet(string $greeting, string $name = "world", string $punct = "!") {
    return "$greeting, $name$punct";
}
echo greet("Hello"), "\n";
echo greet(greeting: "Hi"), "\n";
echo greet("Hey", punct: "?"), "\n";
echo greet(name: "Ann", greeting: "Yo"), "\n";
echo greet(...['greeting' => 'Spread', 'punct' => '.']), "\n";
echo greet("Mixed", ...['punct' => ';']), "\n";

function collect($first, ...$rest) { var_dump($first, $rest); }
collect(1, 2, 3);
collect(1, extra: 'x', more: 'y');
collect(first: 0, x: 1);

function byref(&$out, $v = 1) { $out = $v * 2; }
byref(v: 21, out: $result);
var_dump($result);

class Point {
    public function __construct(public int $x = 0, public int $y = 0, public int $z = 0) {}
    public static function make(int ...$coords): static { return new static(...$coords); }
    public function moved(int $dx = 0, int $dy = 0): static { return new static($this->x + $dx, $this->y + $dy, $this->z); }
}
var_dump(new Point(y: 5));
var_dump((new Point(1, 2))->moved(dy: 10));
var_dump(Point::make(x: 1, z: 3));
$f = fn(int $a, int $b = 10) => $a - $b;
var_dump($f(b: 1, a: 5));
var_dump(call_user_func_array('greet', ['greeting' => 'Callback', 'name' => 'arr']));
var_dump(call_user_func_array([new Point(3), 'moved'], ['dy' => 4]));
var_dump(call_user_func('greet', name: 'cuf', greeting: 'Via'));

echo "-- builtins\n";
var_dump(str_pad(string: "x", length: 5, pad_type: STR_PAD_LEFT));
var_dump(str_pad("x", 5, pad_type: STR_PAD_BOTH, pad_string: "-"));
var_dump(array_slice([1, 2, 3, 4], offset: 1, preserve_keys: true));
var_dump(htmlspecialchars("<'a'>", double_encode: false));
var_dump(round(num: 2.555, precision: 2), json_like(array_fill(start_index: 2, count: 2, value: 'v')));
var_dump(in_array(needle: "1", haystack: [1], strict: true), number_format(1234.5, decimals: 1, thousands_separator: ' '));
var_dump(implode(separator: ",", array: [1, 2]), explode(separator: ",", string: "a,b", limit: -1));
$arr = [3, 1, 2];
sort(array: $arr, flags: SORT_STRING);
var_dump($arr);
var_dump(str_replace(search: "a", replace: "b", subject: "aaa", count: $n), $n);
try { throw new Exception(code: 5, message: "named"); } catch (Exception $e) { var_dump($e->getMessage(), $e->getCode()); }
function json_like($v) { return implode(",", array_map(fn($k, $x) => "$k:$x", array_keys($v), $v)); }

echo "-- errors\n";
$tests = [
    fn() => greet(nope: 1),
    fn() => greet("a", greeting: "b"),
    fn() => greet(name: "x"),
    fn() => str_pad(length: 3),
    fn() => str_pad("x", 3, nope: 1),
    fn() => sprintf("%s", values: 1),
    fn() => greet(...['greeting' => 'a', 'greeting2' => 'b']),
    fn() => collect(1, ...['a' => 1], ...[2]),
    fn() => strlen(string: "abc"),
    fn() => array_keys(array: [1, 2], filter_value: 2),
];
foreach ($tests as $i => $t) {
    try { var_dump($t()); } catch (Throwable $e) { echo $i, ": ", get_class($e), ": ", $e->getMessage(), "\n"; }
}
function f($a, $b = 2, $c = 3) { var_dump(func_get_args(), func_num_args()); }
f(1, c: 5);
function g(int $x) { throw new Exception("t"); }
try { g(x: 1); } catch (Exception $e) { echo $e->getTraceAsString(), "\n"; }
class M { function __call($n, $a) { var_dump($n, $a); } }
(new M)->foo(1, bar: 2);
function h(array $options = [], ?int $limit = null) { var_dump($options, $limit); }
h(limit: 3);
