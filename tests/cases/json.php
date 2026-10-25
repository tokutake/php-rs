<?php
// json_encode / json_decode and friends

echo "-- scalars\n";
foreach ([null, true, false, 0, -42, PHP_INT_MAX, 1.0, 0.1, 1e100, -0.0, 3.14159, "", "abc", "é", "a/b", "\"\\\n\t\x01", "\u{1F600}"] as $v) {
    echo json_encode($v), "\n";
}

echo "-- arrays\n";
echo json_encode([]), "\n";
echo json_encode([1, 2, 3]), "\n";
echo json_encode([1 => 'a', 2 => 'b']), "\n";
echo json_encode(['a' => 1, 'b' => [true, null]]), "\n";
echo json_encode([[], (object)[]]), "\n";
echo json_encode([3 => 1], JSON_FORCE_OBJECT), "\n";
echo json_encode([1, 2], JSON_FORCE_OBJECT), "\n";

echo "-- flags\n";
$s = "<a href='x'>&\"é/\u{1F600}";
echo json_encode($s), "\n";
echo json_encode($s, JSON_HEX_TAG | JSON_HEX_APOS | JSON_HEX_QUOT | JSON_HEX_AMP), "\n";
echo json_encode($s, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES), "\n";
echo json_encode("\u{2028}\u{2029}", JSON_UNESCAPED_UNICODE), "\n";
echo json_encode("\u{2028}", JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_LINE_TERMINATORS), "\n";
echo json_encode(["12", "1.5", "1e3", "abc", " 1"], JSON_NUMERIC_CHECK), "\n";
echo json_encode(10.0), " ", json_encode(10.0, JSON_PRESERVE_ZERO_FRACTION), "\n";
echo json_encode(0.30000000000000004), "\n";

echo "-- pretty\n";
echo json_encode(['a' => 1, 'b' => [1, 2, ['c' => null]], 'd' => [], 'e' => new stdClass], JSON_PRETTY_PRINT), "\n";

echo "-- objects\n";
class P {
    public $pub = 1;
    protected $prot = 2;
    private $priv = 3;
    public $arr = ['x' => 'y'];
}
echo json_encode(new P), "\n";
$o = new stdClass;
$o->{'0'} = 'zero';
$o->name = 'n';
echo json_encode($o), "\n";

class J implements JsonSerializable {
    public function __construct(private $v) {}
    public function jsonSerialize(): mixed { return $this->v; }
}
echo json_encode([new J(5), new J(['k' => new J('nested')]), new J(null)]), "\n";
class Self_ implements JsonSerializable {
    public $a = 1;
    public function jsonSerialize(): mixed { return $this; }
}
echo json_encode(new Self_), "\n";
class Thrower implements JsonSerializable {
    public function jsonSerialize(): mixed { throw new RuntimeException("nope"); }
}
try {
    json_encode([1, new Thrower]);
} catch (RuntimeException $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

enum Suit: string { case Hearts = 'H'; case Spades = 'S'; }
enum Num: int { case One = 1; }
enum Plain { case A; }
echo json_encode([Suit::Hearts, Num::One]), "\n";
var_dump(json_encode(Plain::A));
echo json_last_error(), " ", json_last_error_msg(), "\n";

echo "-- errors\n";
var_dump(json_encode("\xff"));
echo json_last_error(), " ", json_last_error_msg(), "\n";
var_dump(json_encode("a\xffb", JSON_INVALID_UTF8_IGNORE));
var_dump(json_encode("a\xffb", JSON_INVALID_UTF8_SUBSTITUTE));
var_dump(json_encode(["ok", "\xff"], JSON_PARTIAL_OUTPUT_ON_ERROR));
var_dump(json_encode(NAN));
echo json_last_error_msg(), "\n";
var_dump(json_encode(INF, JSON_PARTIAL_OUTPUT_ON_ERROR));
$r = [1];
$r[] = &$r;
var_dump(json_encode($r));
echo json_last_error_msg(), "\n";
$c = new stdClass;
$c->self = $c;
var_dump(json_encode($c));
echo json_last_error_msg(), "\n";
var_dump(json_encode([[[1]]], 0, 2));
echo json_last_error_msg(), "\n";
var_dump(json_encode([[1]], 0, 2));
echo json_last_error(), " ", json_last_error_msg(), "\n";
try {
    json_encode("\xff", JSON_THROW_ON_ERROR);
} catch (JsonException $e) {
    echo get_class($e), " ", $e->getCode(), ": ", $e->getMessage(), "\n";
}
// A thrown error leaves the last error alone
echo json_last_error(), "\n";
try {
    json_encode([], 0, 0);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}

echo "-- decode\n";
foreach (['null', 'true', 'false', '0', '-1', '1.5', '1e2', '-0', '1E-2', '"str"', '"é😀"', '[]', '{}', ' [1, "a", {"b": null}] ', '9223372036854775807', '9223372036854775808', '-9223372036854775809', '12345678901234567890123'] as $j) {
    var_dump(json_decode($j));
}
var_dump(json_decode('{"a":{"b":[1,2]},"0":"zero","":"empty"}'));
var_dump(json_decode('{"a":{"b":[1,2]},"0":"zero","":"empty"}', true));
var_dump(json_decode('{"a":1}', false, 512, JSON_OBJECT_AS_ARRAY));
var_dump(json_decode('{"a":1}', null, 512, JSON_OBJECT_AS_ARRAY));
var_dump(json_decode('{"a":1,"a":2}', true));
var_dump(json_decode('12345678901234567890', false, 512, JSON_BIGINT_AS_STRING));
var_dump(json_decode('[1.0e400]'));

echo "-- decode errors\n";
foreach (['', '[', '[1,]', '{"a"}', "'x'", 'nul', '01', '1.', '"\x01"', '"\ud800"', '"\ud800x"', "\"\xff\"", '[1] x', '{"\u0000a":1}', '"\u0000a"', '[[1]]'] as $j) {
    var_dump(json_decode($j, false, 2));
    echo json_last_error(), " ", json_last_error_msg(), "\n";
}
var_dump(json_decode('{"\u0000a":1}', true));
var_dump(json_decode("\"a\xffb\"", false, 512, JSON_INVALID_UTF8_IGNORE));
var_dump(json_decode("\"a\xffb\"", false, 512, JSON_INVALID_UTF8_SUBSTITUTE));
try {
    json_decode('{', false, 512, JSON_THROW_ON_ERROR);
} catch (JsonException $e) {
    echo $e->getCode(), ": ", $e->getMessage(), "\n";
}

echo "-- validate\n";
foreach (['{"a":[1,2,{"b":null}]}', '[1,', '', '"x"', '[[[]]]'] as $j) {
    var_dump(json_validate($j));
}
var_dump(json_validate('[[[]]]', 2));
echo json_last_error_msg(), "\n";

echo "-- round trip\n";
$data = ['name' => "日本語", 'list' => [1, 2.5, true, null], 'nested' => ['x' => ['y' => 'z']]];
var_dump(json_decode(json_encode($data), true) === $data);
echo json_encode(json_decode('{"a":[],"b":{}}')), "\n";
echo json_encode(json_decode('{"a":[],"b":{}}', true)), "\n";

echo "-- more\n";
$o = json_decode('{"1":"a","b":{"2":3}}');
var_dump($o);
var_dump((array)$o);
var_dump($o->{'1'});
echo json_encode($o), "\n";
echo json_encode([1=>[], 'x'=>new stdClass], JSON_PRETTY_PRINT), "\n";
echo json_encode([[1,[2,[3]]]], JSON_PRETTY_PRINT|JSON_PARTIAL_OUTPUT_ON_ERROR, 2), "\n";
echo json_last_error_msg(), "\n";
class R implements JsonSerializable { public function jsonSerialize(): mixed { return [$this]; } }
var_dump(json_encode(new R));
echo json_last_error_msg(), "\n";
var_dump(json_decode(' '), json_decode('"A\/\b\f\n\r\t"'), json_decode('1e999'), json_decode('-'), json_decode('[1,2', true));
var_dump(json_decode('{"a":1}', true, 1));
echo json_last_error_msg(), "\n";
var_dump(json_encode(1.0E+25), json_encode(-1.5e-7), json_encode(123456789012345678.0));
var_dump(json_encode("\x7f\x00"));
var_dump(json_encode(['a'=>1,'b'=>2], JSON_PRETTY_PRINT | JSON_FORCE_OBJECT));
$a = ['x' => 1]; $b = [$a, $a];
echo json_encode($b), "\n";
var_dump(json_last_error());
json_decode('[');
json_encode(1);
var_dump(json_last_error());
