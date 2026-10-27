<?php
foreach (['int', 'integer', 'float', 'double', 'string', 'bool', 'boolean', 'array', 'null'] as $t) { $v = "12.5abc"; var_dump(settype($v, $t), $v); }
$o = 5; var_dump(settype($o, 'object'), $o);
try { $x = 1; settype($x, 'nope'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
foreach (['42', ' 42 ', '4.2', '0x1A', '-7', '+3', 'abc', '', '9999999999999999999', '012'] as $s) var_dump(filter_var($s, FILTER_VALIDATE_INT));
var_dump(filter_var('5', FILTER_VALIDATE_INT, ['options' => ['min_range' => 1, 'max_range' => 4]]), filter_var('x', FILTER_VALIDATE_INT, ['options' => ['default' => 9]]), filter_var('0x1A', FILTER_VALIDATE_INT, FILTER_FLAG_ALLOW_HEX), filter_var('012', FILTER_VALIDATE_INT, FILTER_FLAG_ALLOW_OCTAL));
foreach (['1.5', '1e3', '.5', 'abc', ' 2 ', '1,000.5'] as $s) var_dump(filter_var($s, FILTER_VALIDATE_FLOAT));
var_dump(filter_var('1,000.5', FILTER_VALIDATE_FLOAT, FILTER_FLAG_ALLOW_THOUSAND));
foreach (['yes', 'on', '1', 'true', 'no', 'off', '0', 'false', '', 'maybe', ' TRUE '] as $s) var_dump(filter_var($s, FILTER_VALIDATE_BOOLEAN), filter_var($s, FILTER_VALIDATE_BOOL, FILTER_NULL_ON_FAILURE));
foreach (['a@b.co', 'bad@', 'x.y+z@sub.example.org', 'a@b', '"q"@x.com', 'a..b@x.com'] as $s) var_dump(filter_var($s, FILTER_VALIDATE_EMAIL));
foreach (['http://example.com', 'https://a.b/c?d=e#f', 'example.com', 'ftp://x', 'http://', 'javascript:alert(1)', 'http://exa mple.com', 'mailto:a@b.c'] as $s) var_dump(filter_var($s, FILTER_VALIDATE_URL));
var_dump(filter_var('http://x.com', FILTER_VALIDATE_URL, FILTER_FLAG_PATH_REQUIRED), filter_var('http://x.com/p', FILTER_VALIDATE_URL, FILTER_FLAG_QUERY_REQUIRED));
foreach (['127.0.0.1', '256.1.1.1', '::1', '2001:db8::1', '1.2.3', '10.0.0.1', '192.168.1.1', 'fe80::1'] as $s) var_dump(filter_var($s, FILTER_VALIDATE_IP), filter_var($s, FILTER_VALIDATE_IP, FILTER_FLAG_IPV4), filter_var($s, FILTER_VALIDATE_IP, FILTER_FLAG_NO_PRIV_RANGE | FILTER_FLAG_NO_RES_RANGE));
var_dump(filter_var(' x <b> ', FILTER_DEFAULT), filter_var(123, FILTER_DEFAULT), filter_var('a-b', FILTER_VALIDATE_REGEXP, ['options' => ['regexp' => '/^a/']]), filter_var(['1', 'x'], FILTER_VALIDATE_INT, FILTER_REQUIRE_ARRAY), filter_var('5', FILTER_VALIDATE_INT, FILTER_REQUIRE_ARRAY));
var_dump(filter_var('<b>hi</b> & "q"', FILTER_SANITIZE_SPECIAL_CHARS), filter_var('a1b2.3', FILTER_SANITIZE_NUMBER_INT), filter_var('a1b2.3e', FILTER_SANITIZE_NUMBER_FLOAT, FILTER_FLAG_ALLOW_FRACTION), filter_var('a@b().c', FILTER_SANITIZE_EMAIL), filter_var('<p>x</p>', FILTER_SANITIZE_FULL_SPECIAL_CHARS), filter_var('ab cd', FILTER_SANITIZE_ENCODED), filter_var('x', FILTER_CALLBACK, ['options' => 'strtoupper']));
var_dump(escapeshellarg("it's"), escapeshellarg(''), escapeshellcmd("ls; rm -rf / 'x' \"y\" #z"));
var_dump(setlocale(LC_ALL, 'C'), setlocale(LC_CTYPE, 'xx_YY'));
$z = timezone_identifiers_list(); var_dump(count($z) > 400, $z[0], in_array('Asia/Tokyo', $z), in_array('UTC', $z), end($z));
var_dump(count(timezone_identifiers_list(DateTimeZone::EUROPE)) > 50, timezone_identifiers_list(DateTimeZone::UTC));
var_dump(iconv('UTF-8', 'ISO-8859-1', "café"), bin2hex(iconv('ISO-8859-1', 'UTF-8', "caf\xe9")), iconv('UTF-8', 'ASCII//TRANSLIT', "café"), iconv('UTF-8', 'ASCII//IGNORE', "café"), iconv_strlen("café"), iconv('UTF-8', 'UTF-8//IGNORE', "a\xffb"));
var_dump(sscanf("age: 25 name: Bob", "age: %d name: %s"), sscanf("12 apples", "%d %s"), sscanf("x", "%d"), sscanf("3.5kg", "%f%s"), sscanf("abc", "%2s"), sscanf("0x1f", "%x"));
