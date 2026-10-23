<?php
// Literals and keys
$a = [1, 2, 3];
$b = ['x' => 1, 'y' => [10, 20], 5 => 'five', 'six'];
$c = array(-5 => 'a', 'b');
var_dump($a, $b, $c);
var_dump(["1" => 'x', "01" => 'y', true => 'z', "-0" => 'm', "-3" => 'k', 2.0 => 'f']);
print_r($b); echo "\n";
print_r([1, 'a' => [2, [3]], 'e' => []]);
echo print_r([1], true), "|\n";

// Read and write
echo $a[0], $b['y'][1], $b[6], "\n";
$a[] = 4; $a[10] = 11; $a[] = 12; $a['k'] = 'v';
print_r($a);
$m = []; $m['a']['b'][] = 1; $m['a']['b'][] = 2; $m['a']['c'] = 'z';
var_dump($m);
$n = null; $n[] = 'auto'; var_dump($n);
$a[0] += 100; $a[1] .= "x"; $a[2]++; --$a[3];
var_dump($a[0], $a[1], $a[2], $a[3]);

// Copy-on-write value semantics
$orig = [1, [2, 3]];
$copy = $orig;
$copy[1][0] = 'changed';
$copy[] = 'new';
print_r($orig); print_r($copy);

// isset / empty / ?? / unset
$h = ['a' => 1, 'n' => null, 'z' => 0, 'arr' => ['deep' => 'yes']];
var_dump(isset($h['a']), isset($h['n']), isset($h['missing']), isset($h['arr']['deep']), isset($h['arr']['no']['x']), isset($undef), isset($undef['x']));
var_dump(empty($h['z']), empty($h['a']), empty($h['missing']), empty($undef));
echo $h['missing'] ?? 'default', " ", $h['arr']['deep'] ?? 'no', " ", $undef['x']['y'] ?? 'deep default', "\n";
unset($h['a'], $h['arr']['deep'], $h['nope']);
var_dump($h);
$h['q'] ??= 'set'; $h['q'] ??= 'not set'; echo $h['q'], "\n";

// foreach
foreach ([10, 20, 30] as $v) echo $v, " ";
echo "\n";
foreach (['a' => 1, 'b' => 2] as $k => $v) { echo "$k=$v "; }
echo "\n";
$arr = [1, 2, 3];
foreach ($arr as $v) { $arr[] = $v * 10; }
print_r($arr);
$pairs = [];
foreach ([[1, 2], [3, 4]] as $i => $pair) { foreach ($pair as $x) { if ($x == 3) continue 2; $pairs[] = "$i:$x"; } }
echo implode(",", $pairs), "\n";
foreach ([1, 2, 3] as $k => $out['last']) {}
var_dump($k, $out);

// Interpolation
$user = ['name' => 'Alice', 'tags' => ['php', 'rust']];
$i = 1;
echo "Name: $user[name], tag: {$user['tags'][1]}, idx: {$user['tags'][$i]}, first: $a[0]\n";
$s = "hello";
echo $s[0], $s[-1], $s[1], "{$s[4]}", "\n";
$s[0] = 'J'; $s[7] = '!'; var_dump($s);

// Operators
var_dump([1, 2] == [1 => 2, 0 => 1], [1, 2] === [1 => 2, 0 => 1], [1, 2] === [1, 2], [1, 2] < [1, 3]);
var_dump([1, 2] < ['a' => 1, 'b' => 2], [1, 2] > ['a' => 1, 'b' => 2], [1, 2] <=> ['a' => 1, 'b' => 2], [] == false, [0] > 99999);
var_dump([1, 2] + [5, 6, 7], ['a' => 1] + ['a' => 2, 'b' => 3]);
var_dump((bool)[], (bool)[0], (int)[5, 6], count([1, [2, 3]]));
