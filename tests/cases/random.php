<?php
mt_srand(42);
echo mt_rand(), " ", mt_rand(1, 100), " ", mt_rand(-5, 5), " ", rand(), " ", rand(10, 1), "\n";
echo mt_rand(0, PHP_INT_MAX), " ", mt_rand(PHP_INT_MIN, PHP_INT_MAX), "\n";
$a = range(1, 10); shuffle($a); echo implode(",", $a), "\n";
echo str_shuffle("abcdefghij"), "\n";
var_dump(array_rand(['a'=>1,'b'=>2,'c'=>3]), array_rand(range(0,9), 3), array_rand(range(0,9), 8));
srand(5); echo rand(), " ", mt_rand(1,6), "\n";
mt_srand(3, MT_RAND_PHP); echo mt_rand(), " ", mt_rand(1, 100), "\n";
echo mt_getrandmax(), getrandmax(), "\n";
try { array_rand([]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { array_rand([1,2], 3); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { mt_rand(5, 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { random_int(5, 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { random_bytes(0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { mt_rand(5); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
var_dump(strlen(random_bytes(16)), random_int(3, 3), lcg_value() < 1, strlen(uniqid()), strlen(uniqid('', true)));
$r = random_int(-3, 3); var_dump($r >= -3 && $r <= 3, is_int(random_int(PHP_INT_MIN, PHP_INT_MAX)));
// The same seed gives the same sequence
mt_srand(1234); $a = [mt_rand(), mt_rand(1, 10), str_shuffle("hello world")];
mt_srand(1234); var_dump($a === [mt_rand(), mt_rand(1, 10), str_shuffle("hello world")]);
mt_srand(7); $x = ['a' => 1, 'b' => 2, 'c' => 3, 10 => 4]; shuffle($x); print_r($x);
var_dump(array_rand(['only' => 1]), array_rand(range(1, 6), 6));
mt_srand(99); for ($i = 0; $i < 700; $i++) { $last = mt_rand(); } echo $last, "\n";
