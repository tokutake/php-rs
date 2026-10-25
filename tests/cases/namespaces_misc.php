<?php
namespace A;
use function X\f;
use const X\C;
use X\{Y, Z as W,};
echo "ok\n";
function g() {}
class K {}
var_dump(new K, \A\g(), namespace\g());
use function strlen;
 echo strlen("ab"), "\n";