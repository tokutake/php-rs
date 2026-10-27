<?php
$urls = ["http://user:pw@example.com:8080/path/to?x=1&y=2#frag", "https://example.com", "//example.com/p", "/just/path?q", "mailto:joe@example.com",
 "example.com:80", "a.com:80/x", "file:///etc/passwd", "file:///c:/dir/f.txt", "http://[::1]:80/x", "http://host:99999/", "http:///x", "?q=1#", "#",
 "", ":", ":80", "http://a:b@c:d@e/", "foo:bar", "http://h/p#f?q", "http://h?#", "x:/y", "http://host:/p", "http://ex\x01ample.com/a b", "abc:123456/x", "1http://x", "http://u@h", "//h:8", "h:8/x?y"];
foreach ($urls as $u) { echo json_encode($u), " => "; var_export(parse_url($u)); echo "\n"; }
var_dump(parse_url("http://h:81/p?q#f", PHP_URL_PORT), parse_url("http://h/p", PHP_URL_QUERY), parse_url("http://h/p", PHP_URL_HOST));
try { parse_url("x", 9); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$s = "a b+c~d.e_f-g/h?i=j&k\xff\0é";
echo urlencode($s), "\n", rawurlencode($s), "\n", urldecode("a+b%20c%zz%4"), "\n", rawurldecode("a+b%20c%41%4g"), "\n";
foreach (["", "f", "fo", "foo", "foob", "fooba", "foobar", "\xff\xfe\x00"] as $x) echo base64_encode($x), "|";
echo "\n";
foreach (["Zm9vYg==", "Zm9vYg", "Zm9v Yg==\n", "Zm9vYg=", "Zm9vY", "Zm9v!Yg==", "Zm9vYg==Zg", "Z", "Zm9vYg===", "=Zm9v"] as $x) { var_dump(base64_decode($x), base64_decode($x, true)); }
echo http_build_query(['a' => 1, 'b' => 'x y', 'c' => [1, 2, 'k' => ['z' => true]], 'd' => null, 'e' => false, 'f' => 1.5, 3 => 'n']), "\n";
echo http_build_query([1, 2, 'x' => [3]], 'p_'), "\n";
echo http_build_query(['a b' => 'c d~'], '', '&amp;', PHP_QUERY_RFC3986), "\n";
class P { public $a = 1; protected $b = 2; private $c = 3; public $d = ['x' => 'y']; }
echo http_build_query(new P), "\n";
echo http_build_query(['o' => new P]), "\n";
enum S: string { case A = 'aa'; } enum U { case X; }
echo http_build_query(['e' => S::A]), "\n";
try { http_build_query(['e' => U::X]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { http_build_query(S::A); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$r = ['a' => 1]; $r['self'] = &$r; echo http_build_query($r), "\n";
