<?php
$inputs = ["", "abc", "The quick brown fox jumps over the lazy dog", str_repeat("a", 55), str_repeat("b", 56), str_repeat("c", 64), str_repeat("xyz", 100), str_repeat("q", 111), str_repeat("r", 112), "\0\xff\x80"];
$algos = ['md5', 'sha1', 'sha224', 'sha256', 'sha384', 'sha512/224', 'sha512/256', 'sha512', 'sha3-224', 'sha3-256',
    'sha3-384', 'sha3-512', 'adler32', 'crc32', 'crc32b', 'crc32c', 'fnv132', 'fnv1a32', 'fnv164', 'fnv1a64', 'joaat'];
foreach ($algos as $a) {
    foreach ($inputs as $i) echo $a, " ", hash($a, $i), "\n";
}
foreach (array_slice($algos, 0, 12) as $a) {
    echo $a, " ", hash_hmac($a, "data", "key"), " ", hash_hmac($a, "data", str_repeat("k", 200)), "\n";
}
echo md5("hello"), " ", sha1("hello"), " ", bin2hex(md5("hello", true)), " ", strlen(sha1("x", true)), "\n";
echo hash("SHA256", "x"), "\n";
var_dump(hash_equals("abc", "abc"), hash_equals("abc", "abd"), hash_equals("abc", "ab"));
try { hash("nope", "x"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { hash_hmac("crc32b", "x", "k"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { hash_equals(1, "x"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$f = sys_get_temp_dir() . "/php-rs-hash-" . getmypid() . ".txt";
file_put_contents($f, "file data");
echo md5_file($f), " ", sha1_file($f), " ", hash_file("crc32b", $f), " ", hash_hmac_file("md5", $f, "k"), "\n";
var_dump(@md5_file("/nonexistent/file.txt"));
unlink($f);
