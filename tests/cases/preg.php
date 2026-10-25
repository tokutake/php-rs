<?php
// preg_* functions and the PCRE2-compatible engine

echo "-- functions\n";
// preg_match details
var_dump(preg_match('/b/', 'abc', $m, PREG_OFFSET_CAPTURE, 1), $m);
var_dump(preg_match('/b/', 'abc', $m, 0, 2), $m);
var_dump(preg_match('/b/', 'abc', $m, 0, -2), $m);
var_dump(preg_match('/b/', 'abc', $m, 0, -10), $m);
var_dump(preg_match('/b/', 'abc', $m, 0, 4), $m, preg_last_error_msg());
var_dump(preg_match('/^b/', 'abc', $m, 0, 1));
var_dump(preg_match('/\Gb/', 'abc', $m, 0, 1));
var_dump(preg_match('/(?<=a)b/', 'abc', $m, 0, 1));
var_dump(preg_match('/(a)(x)?(c)?/', 'ab', $m), $m);
var_dump(preg_match('/(a)(x)?(c)?/', 'ab', $m, PREG_UNMATCHED_AS_NULL), $m);
var_dump(preg_match('/(a)(x)?(c)?/', 'ab', $m, PREG_UNMATCHED_AS_NULL | PREG_OFFSET_CAPTURE), $m);
var_dump(preg_match('/(?<a>x)?(?<b>y)/', 'y', $m), $m);
var_dump(preg_match('/(?<a>x)?(?<b>y)?/', 'z', $m, PREG_UNMATCHED_AS_NULL), $m);
var_dump(preg_match('/(?J)(?<n>a)|(?<n>b)/', 'b', $m), $m);
try { preg_match('/a/', 'a', $m, PREG_SET_ORDER); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { preg_match_all('/a/', 'a', $m, 3); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$m = 'untouched'; var_dump(preg_match('/(/', 'a', $m), $m);
var_dump(preg_match('/a/u', "\xff", $m), $m, preg_last_error(), preg_last_error_msg());
var_dump(preg_match('/a/u', "é", $m, 0, 1), preg_last_error_msg());
var_dump(preg_match('/a/', "\xff a", $m), $m);
// preg_match_all
var_dump(preg_match_all('/(\d)(\w)?/', 'a1b2 3', $m), $m);
var_dump(preg_match_all('/(?<d>\d)/', 'a1b2', $m), $m);
var_dump(preg_match_all('/(?<d>\d)/', 'a1b2', $m, PREG_SET_ORDER), $m);
var_dump(preg_match_all('/(\d)|(x)/', 'x1', $m, PREG_PATTERN_ORDER | PREG_OFFSET_CAPTURE), $m);
var_dump(preg_match_all('/(\d)|(x)/', 'x1', $m, PREG_SET_ORDER | PREG_UNMATCHED_AS_NULL), $m);
var_dump(preg_match_all('/z/', 'abc', $m), $m);
var_dump(preg_match_all('/z/', 'abc', $m, PREG_SET_ORDER), $m);
var_dump(preg_match_all('/\d/', '123'));
var_dump(preg_match_all('//u', 'aé日'));
var_dump(preg_match_all('/(*MARK:x)a|(*MARK:y)b/', 'ab', $m), $m);
var_dump(preg_match_all('/a/', 'aaa', $m, 0, 1), $m);
// preg_replace
var_dump(preg_replace('/a/', 'b', 'aaa', 2, $c), $c);
var_dump(preg_replace('/a/', 'b', 'aaa', 0, $c), $c);
var_dump(preg_replace(['/a/', '/b/'], ['b', 'c'], 'ab'));
var_dump(preg_replace(['/a/', '/b/'], 'x', 'ab'));
var_dump(preg_replace(['/a/', '/b/'], ['1'], 'ab'));
var_dump(preg_replace('/a/', 'b', ['x' => 'a', 'y' => 'c', 5 => 'aa'], -1, $c), $c);
var_dump(preg_filter('/a/', 'b', ['x' => 'a', 'y' => 'c', 5 => 'aa'], -1, $c), $c);
var_dump(preg_filter('/a/', 'b', 'c'), preg_filter('/a/', 'b', 'a'));
var_dump(preg_replace('/(a)(b)?/', '[\1|$2|${1}x|\\\\1|\$1|$11|${12}|\9]', 'ab a'));
var_dump(preg_replace('/(.)/', '\\0\\', 'ab'));
var_dump(preg_replace('/x/', '$', 'axb'), preg_replace('/x/', '${', 'axb'), preg_replace('/x/', '${1', 'axb'));
var_dump(preg_replace('/(/', 'b', 'a'), preg_last_error());
var_dump(preg_replace(['/a/', '/(/'], 'b', 'a'));
var_dump(preg_replace('/a/u', 'b', "\xff"), preg_last_error());
var_dump(preg_replace('//', '-', 'ab'), preg_replace('/b*/', '-', 'abc'), preg_replace('/(?=b)/', '-', 'abb'));
var_dump(preg_replace('//u', '-', 'é日'));
try { preg_replace('/a/', ['x'], 'a'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(preg_replace('/1/', 2, 213));
var_dump(preg_replace('/a/', 'b', null));
// callbacks
var_dump(preg_replace_callback('/(\w)(\d)?/', function ($m) { return json_encode($m); }, 'a1 b'));
var_dump(preg_replace_callback('/(\w)(\d)?/', fn($m) => json_encode($m), 'a1 b', -1, $c, PREG_UNMATCHED_AS_NULL | PREG_OFFSET_CAPTURE), $c);
var_dump(preg_replace_callback('/(?<w>\w)/', fn($m) => strtoupper($m['w']), 'ab', 1, $c), $c);
var_dump(preg_replace_callback(['/a/', '/b/'], fn($m) => $m[0] . $m[0], 'ab'));
var_dump(preg_replace_callback('/a/', fn($m) => 5, ['k' => 'a', 'b']));
var_dump(preg_replace_callback('/a/', 'strtoupper', 'abc'));
try { preg_replace_callback('/a/', 'nope', 'a'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { preg_replace_callback('/a/', fn($m) => throw new Exception('boom'), 'a'); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
var_dump(preg_replace_callback_array(['/a/' => fn($m) => 'b', '/b/' => fn($m) => 'c'], 'ab', -1, $c), $c);
var_dump(preg_replace_callback_array(['/a/' => fn($m) => 'x'], ['a', 'b']));
try { preg_replace_callback_array([1 => fn($m) => 'x'], 'a'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { preg_replace_callback_array(['/a/' => 'nope'], 'a'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(preg_replace_callback_array(['/(/' => fn($m) => 'x'], 'a'));
// split
var_dump(preg_split('/,/', 'a,b,,c', 2));
var_dump(preg_split('/,/', 'a,b,,c', 0));
var_dump(preg_split('/,/', 'a,b,,c', 1));
var_dump(preg_split('/,/', 'a,b,,c', -5));
var_dump(preg_split('/,/', 'a,b,,c,', -1, PREG_SPLIT_NO_EMPTY));
var_dump(preg_split('/(,)/', 'a,b', -1, PREG_SPLIT_DELIM_CAPTURE | PREG_SPLIT_OFFSET_CAPTURE));
var_dump(preg_split('/(,)(x)?/', 'a,b', -1, PREG_SPLIT_DELIM_CAPTURE));
var_dump(preg_split('/(,)(x)?/', 'a,b', -1, PREG_SPLIT_DELIM_CAPTURE | PREG_SPLIT_NO_EMPTY));
var_dump(preg_split('//', 'abc'));
var_dump(preg_split('//', 'abc', 2));
var_dump(preg_split('//u', 'é日', -1, PREG_SPLIT_NO_EMPTY | PREG_SPLIT_OFFSET_CAPTURE));
var_dump(preg_split('/x*/', 'axbc'));
var_dump(preg_split('/\d/', ''), preg_split('/\d/', '', -1, PREG_SPLIT_NO_EMPTY));
var_dump(preg_split('/(/', 'a'));
var_dump(preg_split('/a/u', "\xff"), preg_last_error());
// grep / quote / errors
var_dump(preg_grep('/\d/', ['a', 'b1', 3 => 'c2', 'x' => 5]));
var_dump(preg_grep('/\d/', ['a', 'b1', 3 => 'c2', 'x' => 5], PREG_GREP_INVERT));
var_dump(preg_grep('/a/u', ['a', "\xff", 'a']), preg_last_error());
var_dump(preg_quote(''), preg_quote("a\0b"), preg_quote('a#b/c', '#'), preg_quote('-x-', 'xyz'), preg_quote('.+*?[^]$(){}=!<>|:-#/'));
var_dump(preg_last_error(), preg_last_error_msg());
var_dump(PREG_PATTERN_ORDER, PREG_SET_ORDER, PREG_OFFSET_CAPTURE, PREG_UNMATCHED_AS_NULL, PREG_SPLIT_NO_EMPTY, PREG_SPLIT_DELIM_CAPTURE, PREG_SPLIT_OFFSET_CAPTURE, PREG_GREP_INVERT, PREG_NO_ERROR, PREG_INTERNAL_ERROR, PREG_BACKTRACK_LIMIT_ERROR, PREG_RECURSION_LIMIT_ERROR, PREG_BAD_UTF8_ERROR, PREG_BAD_UTF8_OFFSET_ERROR, PREG_JIT_STACKLIMIT_ERROR);

echo "-- patterns\n";
$cases = [
 ['/abc/', 'xabcabc'], ['/a.c/', "a\nc abc"], ['/a.c/s', "a\nc"], ['/^abc$/m', "x\nabc\ny"], ['/^/m', "a\nb\n"], ['/$/m', "a\nb\n"],
 ['/$/', "ab\n"], ['/$/D', "ab\n"], ['/\Z/', "ab\n"], ['/\z/', "ab\n"], ['/\A./', "ab"], ['/\G\w/', "ab cd"],
 ['/a*/', 'baaa'], ['/a*?/', 'aaa'], ['/a+?b/', 'aaab'], ['/a{2,3}/', 'aaaaaaa'], ['/a{2,}/', 'aaaaa a aa'], ['/a{0}b/', 'ab'],
 ['/a{2,3}?/', 'aaaa'], ['/(ab)+/', 'ababab x ab'], ['/(a|b)*c/', 'ababc'], ['/(a*)*b/', 'aab'], ['/(a*)+b/', 'b'], ['/(a|)+b/', 'aab'],
 ['/x{,3}/', 'x{,3}'], ['/a{/', 'a{'], ['/a{1/', 'a{1'], ['/a{1,2/', 'a{1,2'],
 ['/[abc]+/', 'xxbcay'], ['/[^abc]+/', 'abxyzc'], ['/[a-z0-9_]+/', 'Hello_World 42'], ['/[]a]+/', ']a]b'], ['/[^]a]/', ']ab'], ['/[a-]+/', 'a-a-b'],
 ['/[\d\s]+/', 'ab 12 3c'], ['/[\w.-]+@[\w.-]+/', 'mail me at foo.bar-baz@ex-ample.com!'], ['/[[:alpha:]]+/', 'abc123def'], ['/[[:^digit:]]+/', 'ab12cd'],
 ['/[[:punct:]]+/', 'a!@#b'], ['/[[:space:][:upper:]]+/', 'aB C d'], ['/[\x41-\x43]+/', 'ABCD'], ['/[\101]/', 'A'], ['/\x41\101\x{42}/', 'AAB'],
 ['/\d+/', 'a12b345'], ['/\D+/', 'a12b345'], ['/\s+/', "a \t\nb"], ['/\S+/', "a \t\nb"], ['/\w+/', 'héllo wörld'], ['/\W+/', 'a, b!'],
 ['/\h+/', "a \t b"], ['/\v+/', "a\n\r\x0bb"], ['/\R/', "a\r\nb\nc\rd"], ['/\N+/', "ab\ncd"], ['/\bw\w*/', 'what world wide'], ['/\Bo/', 'foo o'],
 ['/(?i)abc/', 'xAbC'], ['/a(?i)b/', 'aB AB'], ['/a(?i:b)c/', 'aBc aBC'], ['/(?i)a(?-i)b/', 'Ab AB'], ['/(a(?i)b|c)/', 'aB C c'],
 ['/(?x) a b c # comment
 /', 'abc'], ['/a b/x', 'ab a b'], ['/[a b]/x', ' '], ['/(?s).+/', "a\nb"], ['/(?m)^b/', "a\nb"],
 ['/(\w)\1/', 'hello'], ['/(?<c>\w)\k<c>/', 'aabb'], ['/(?P<c>\w)(?P=c)/', 'xyzzy'], ['/(\w)\g1/', 'abcc'], ['/(\w)\g{-1}/', 'abcc'], ['/(a)|\1b/', 'ab'],
 ['/(?i)(a)\1/', 'aA'], ['/(?:a|b)+/', 'abba'], ['/(?|(a)|(b))/', 'b'], ['/(?|(a)(x)|(b))\2/', 'axx'], ['/(?>a+)b/', 'aaab'], ['/(?>a+)ab/', 'aaab'],
 ['/a++b/', 'aaab'], ['/a++a/', 'aaa'], ['/a*+/', 'aaa'], ['/(?:ab)++c/', 'ababc'], ['/\d?+5/', '5'],
 ['/foo(?=bar)/', 'foobar foobaz'], ['/foo(?!bar)/', 'foobar foobaz'], ['/(?<=\$)\d+/', 'cost $42 or 7'], ['/(?<!\$)\b\d+/', 'cost $42 or 7'],
 ['/(?<=a|bc)x/', 'ax bcx cx'], ['/(?<=(?:a|b)c)d/', 'acd bcd cd'], ['/(?=(\w+))a/', 'abc'], ['/(?!a)\w/', 'abc'],
 ['/^(?:(\d+)|(\w+))$/', 'abc'], ['/(a)?b/', 'b'], ['/(a)?(b)?c/', 'bc'], ['/(a)|(b)/', 'b'],
 ['/(?(1)a|b)/', 'ab'], ['/(a)?(?(1)b|c)/', 'ab c'], ['/(?(?=a)ab|cd)/', 'ab cd'], ['/(?<n>x)?(?(<n>)y|z)/', 'xy z'], ['/(?(?<=a)b|c)/', 'ab c'],
 ['/^(\((?:[^()]++|(?1))*\))$/', '((a)(b(c)))'], ['/(?R)?x/', 'xxx'], ['/(a|b(?1))/', 'bba'], ['/(?<p>a|\((?&p)\))/', '((a))'], ['/(?(DEFINE)(?<d>\d+))(?&d)-(?&d)/', '12-34'],
 ['/a\Kb/', 'ab ab'], ['/(?=ab\K)/', 'ab'], ['/a(*FAIL)|b/', 'ab'], ['/a(*F)|b/', 'ab'], ['/(*MARK:A)x|(*MARK:B)y/', 'y x'],
 ['/a+(*COMMIT)b/', 'aaac aaab'], ['/a+(*PRUNE)b/', 'aaac aaab'], ['/aaa(*SKIP)b|a+/', 'aaac'], ['/\w+(*SKIP)(*FAIL)|\d/', 'ab 1'],
 ['/./u', 'héllo'], ['/\w+/u', 'héllo wörld'], ['/[à-ü]+/u', 'àéü x'], ['/\p{L}+/u', 'abc日本語123'], ['/\p{Han}+/u', 'abc日本語ひらがな'], ['/\p{Hiragana}+/u', 'abc日本語ひらがな'],
 ['/\P{L}+/u', 'abc123'], ['/\p{^L}+/u', 'abc123'], ['/\pL+/u', 'ab1'], ['/\p{Lu}/u', 'aÉb'], ['/\p{Nd}+/u', 'x٣٤5'], ['/\X/u', "e\u{301}x"], ['/\d+/u', '١٢٣ 45'],
 ['/é/iu', 'É'], ['/[é]/iu', 'É'], ['/\x{212A}/iu', 'k'], ['/k/iu', "\u{212A}"], ['/[k]/iu', "\u{212A}"], ['/ß/iu', 'ẞ'], ['/σ/iu', 'Σς'], ['/[a-z]+/iu', 'ÀbcD'],
 ['/(é)\1/iu', 'éÉ'], ['/\bé/u', ' é'], ['/^.$/u', '日'], ['/^.$/', '日'], ['/^...$/', '日'], ['/[^a]/u', '日'], ['/\x{65e5}/u', '日本'],
 ['/(?<=日)本/u', '日本'], ['/[[:alpha:]]+/u', 'été'], ['/[[:punct:]]+/u', '«»!'], ['/\s+/u', "a\u{3000}b"], ['/\h/u', "\u{a0}"],
 ['/a|b|c/', 'xcba'], ['/(a|ab)(c|bcd)(d*)/', 'abcd'], ['/((a)|b)+/', 'ab'], ['/(a+|b+)*c/', 'aabbc'], ['/(?:(a)|b)*/', 'ab'],
 ['/x*/', ''], ['/^$/', ''], ['/a?/', ''], ['/()/', 'ab'], ['/(?:)/', 'a'], ['/\b/', 'ab cd'], ['/(?=a)/', 'aaa'],
 ['/\Qa.b\E+/', 'a.bbb a.b'], ['/[\Q]\E]/', ']'], ['/\./', 'a.b'], ['/\//', 'a/b'], ['#a/b#', 'a/b'], ['{a}', 'a'], ['(a(b)c)', 'abc'], ['[a]', 'a'],
 ['/\t\n\r\f\v\e\a/', "\t\n\r\f\v\x1b\x07"], ['/\0/', "a\0b"], ['/\cA/', "\x01"], ['/\o{101}/', 'A'], ['/\x{0}/', "\0"],
 ['/(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)\10/', 'abcdefghijj'], ['/(a)\10/', "a\x08"], ['/\11/', "\t"],
 ['/a/i', 'A'], ['/[^a]/i', 'A'], ['/[A-C]/i', 'b'], ['/\w+/i', 'ÀB'], ['/(?U)a+/', 'aaa'], ['/a+?/U', 'aaa'], ['/(?n)(a)(?<x>b)/', 'ab'], ['/(a)(b)/n', 'ab'],
 ['/abc/A', 'xabc abc'], ['/b/A', 'ab'], ['/(*UTF)é/', 'é'], ['/(*CRLF)a$/m', "a\r\nb"], ['/(*CR)^b/m', "a\rb"], ['/(*ANYCRLF)a.b/', "a\rb a.b"], ['/(*ANY)a$/m', "a\x0bb"],
 ['/(?J)(?<a>x)|(?<a>y)/', 'y'], ['/(?<a>x)|(?<a>y)/J', 'y'],
];
foreach ($cases as $i => [$p, $s]) {
    $r = preg_match_all($p, $s, $m, PREG_OFFSET_CAPTURE);

    echo "#$i $p => ", json_encode($r), " ", json_encode($m, JSON_INVALID_UTF8_SUBSTITUTE), "\n";
    $r = preg_match($p, $s, $m, PREG_UNMATCHED_AS_NULL);
    echo "   ", json_encode($r), " ", json_encode($m, JSON_INVALID_UTF8_SUBSTITUTE), "\n";
    $r = preg_replace($p, '[$0]', $s);
    echo "   ", json_encode($r, JSON_INVALID_UTF8_SUBSTITUTE), "\n";
    $r = preg_split($p, $s, -1, PREG_SPLIT_DELIM_CAPTURE);
    echo "   ", json_encode($r, JSON_INVALID_UTF8_SUBSTITUTE), "\n";
}

echo "-- compile errors\n";
foreach (["/[:alpha:]/", "/\\b*/", "/^*/", "/(?=a)*/", "/\\K+/", "/a{99999}/", "/(?<1a>x)/", "/(?<a/", "/\\k<zz>/", "/(?(1)a|b|c)/", "/(?z)/", "/\\c/", "/\\i/", "/[\\R]/", "/[z-a]/", "/[a-\\d]/", "/(?#abc/", "/(?<=(a|bc))x/", "/\\x{110000}/u", "/\\o{8}/", "/(*FOO)/", "/(?R/", "/\\g/", "/\\g{0}/","/(?-0)/", "/((((a))))\\5/", "/a)b/", "/(a/", "/[[:foo:]]/", "/\\N{U+41}/", "/\\p/", "/\\p{/", "/\\8/", "/(?<=\\R)a/", "/(?<=\\d+)/", "/a{3,2}b/", "/(?<n>a)(?<n>b)/", "/\xff/u", "/a\xc3/u", "/[a/", "/[]/", "/[^]/", "/x**/", "/(?/", "/(?P/", "/(?Px)/", "/\\/", "/(?<=a(?R))/", "/a(?C1)b/", "/(?(R)a|b)/", "/(?(R1)a|b)/", "/(?(DEFINE)a|b)/", "/(?(?=a)b|c|d)/", "/(?(<x>)a)/", "/\\ga/", "/\\g<a/", "/\\k/", "/(?i/", "/(?-/", "/(?i-/", "/abc\\Q/", "/[\\Qa/", "/\\x{zz}/", "/\\x{}/", "/\\o{}/", "/\\o/", "/\\u1234/", "/\\L/", "/(?|(a)|(?<n>b))/", "/(?<n>a)|(?<m>b)\\k<n>/", "/{1}/", "/a|*/", "/(*:)/", "/(*MARK)/", "/(*ACCEPT:x)/", "/\\cé/", "/[\\N]/", "/(*UTF)\xff/", "/\\p{Han}/", "/\\p{Lu}/", "/\\p{sc:Latin}/u", "/\\p{scx:Latn}/u", "/\\p{Bogus}/u", "/\\p{L&}/u", "/\\p{Xwd}+/u", "/a{1,65536}/", "/(?<abcdefghijabcdefghijabcdefghijabc>x)/", "/x(?#c)*/", "/(?i)*/", "/\\Q\\E*/", "/a\\E*/", "/[a-\\pL]/u", "/[\\pL-z]/u", "/[a-[:digit:]]/", "/[[:digit:]-z]/", "/[[.a.]]/", "/[[=a=]]/"] as $p) { $r = preg_match($p, "abc"); echo $p, " => ", var_export($r, true), " ", preg_last_error(), "\n"; }
foreach (["abc", "", "   ", "\\abc\\", "1abc1", "/abc", "(abc", "{abc}", "[abc]", "<abc>", "(a(b)c)", "/a/ix m", "/a/Z", "#a#u", "~a~", "/a\\/b/", "{a{b}c}", "{a\\}b}", "/a/S", "/a/X"] as $p) { $r = preg_match($p, "abc"); echo json_encode($p), " => ", var_export($r, true), "\n"; }
