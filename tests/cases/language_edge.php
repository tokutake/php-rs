<?php
var_dump(1 << 3);
$a = ['x' => 'X'];
echo <<<EOT
  multi {$a[
    'x']} end
  EOT;
echo "\n";
echo match (1) { default => "d", 1 => "one" }, "\n";
[$s1, $s2] = "ab";
var_dump($s1);
$i = 0;
$arr = [];
[$arr[$i++], $arr[$i++]] = ['a', 'b'];
var_dump($arr);
switch ("1e1") { case 10: echo "loose\n"; break; }
for ($i = 0; $i < 3; $i++) { switch ($i) { case 1: continue 2; } echo $i; }
echo "\n";
try { echo match (2) {}; } catch (Error $e) { echo $e->getMessage(), "\n"; }
echo match (true) { default => "x" }, "\n";
try { echo match ("a very long string here") { 1 => 2 }; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { echo match ("\x01é") { 1 => 2 }; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { echo match (0.1 + 0.2) { 1 => 2 }; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { echo match (2.0) { 1 => 2 }; } catch (Error $e) { echo $e->getMessage(), "\n"; }
const A = 1;
const A = 2;
echo A, "\n";
var_dump(defined('A'), constant('A'));
class K { const X = 5; }
var_dump(constant('K::X'), defined('K::X'), defined('K::Y'));
$x = [1, 2];
[$p, [$q]] = [1, [2]];
echo "$p$q\n";
