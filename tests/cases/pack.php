<?php
$cases = [['nvc*', 0x1234, 0x5678, 65, 66], ['NV', 0x01020304, 0x01020304], ['a5A5Z5', "ab", "cd", "ef"], ['H*h*', "4a6b", "4a6b"], ['H3', "abc"], ['sSlLqQJP', -1, 65535, -2, 4294967295, -3, PHP_INT_MAX, 1, 2], ['iI', -5, 5], ['fdgGeE', 1.5, 2.25, 3.5, 4.75, 5.5, 6.25], ['x2X1@6C', 7], ['C3', 1, 2, 3], ['a*', "rest"], ['Z*', "zs"], ['A*', "sp  "], ['c', 300], ['n', -1]];
foreach ($cases as $c) { $f = array_shift($c); echo $f, ": ", bin2hex(pack($f, ...$c)), "\n"; }
print_r(unpack('nfirst/vsecond/c2chars', pack('nvc*', 0x1234, 0x5678, 65, -66)));
print_r(unpack('a5x/A5y/Z5z', pack('a5A5Z5', "ab", "cd", "ef")));
print_r(unpack('H*hex', "Jk"));
print_r(unpack('h*', "Jk"));
print_r(unpack('N2', "\x00\x00\x00\x01\x00\x00\x00\x02"));
print_r(unpack('C*', "abc"));
print_r(unpack('q', pack('q', -12345)));
print_r(unpack('J', pack('J', 1 << 40)));
print_r(unpack('f', pack('f', 1.5)));
print_r(unpack('d', pack('d', -2.5)));
print_r(unpack('@2/Cx', "abcd"));
print_r(unpack('C2a/Cb', "\x01\x02\x03"));
print_r(unpack('Cx/Cy', "\x01\x02\x03\x04", 2));
var_dump(unpack('N', "ab"));
try { pack('N'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { pack('y', 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { pack('N', 1, 2); } catch (ValueError $e) { echo get_class($e), $e->getMessage(), "\n"; }
