<?php
$c = hash_init('sha256'); hash_update($c, 'ab'); $d = hash_copy($c); hash_update($c, 'c');
echo hash_final($c), " ", hash('sha256', 'abc'), "\n", hash_final($d), "\n";
$h = hash_init('md5', HASH_HMAC, 'key'); hash_update($h, 'data'); echo hash_final($h), " ", hash_hmac('md5', 'data', 'key'), "\n";
var_dump(get_class($c));
try { hash_update($c, 'x'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { hash_init('md5', HASH_HMAC); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { hash_init('crc32b', HASH_HMAC, 'k'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
echo bin2hex(hash_final(hash_init('crc32b'), true)), "\n";
