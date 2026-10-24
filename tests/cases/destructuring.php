<?php
[$a, $b] = [1, 2];
echo "$a $b\n";
[$a, $b] = [$b, $a];
echo "$a $b\n";
list($x, , $z) = [10, 20, 30];
echo "$x $z\n";
['k' => $k, 'v' => $v] = ['v' => 'vee', 'k' => 'kay'];
echo "$k $v\n";
list('a' => $p, 'b' => list($q, $r)) = ['a' => 1, 'b' => [2, 3]];
echo "$p $q $r\n";
[[$m, $n], [$o]] = [[1, 2], [3]];
echo "$m $n $o\n";
// Missing keys warn and assign null
[$u, $w] = [1];
var_dump($w);
['nope' => $nope] = [];
var_dump($nope);
// Non-array source assigns null silently
[$s1, $s2] = 42;
var_dump($s1, $s2);
// The assignment yields the right-hand side
var_dump([$f] = [7, 8]);
// Into array elements and properties
$arr = [];
[$arr[], $arr['x']] = ['first', 'second'];
var_dump($arr);
$obj = new stdClass;
[$obj->a, $obj->b] = [1, 2];
var_dump($obj);
// By reference
$src = [1, [2, 3]];
[&$ra, [, &$rb]] = $src;
$ra = 100;
$rb = 300;
var_dump($src);
// foreach with destructuring
foreach ([[1, 'one'], [2, 'two']] as [$num, $word]) {
    echo "$num=$word\n";
}
foreach (['x' => ['id' => 5, 'name' => 'five']] as $key => ['id' => $id, 'name' => $name]) {
    echo "$key: $id $name\n";
}
// Nested keys
$data = [['id' => 1, 'tags' => ['a', 'b']]];
foreach ($data as ['id' => $id, 'tags' => [$t1, $t2]]) {
    echo "$id $t1 $t2\n";
}
// String keys on a list
[$zero, $one] = ['a' => 1, 0 => 'z', 1 => 'o'];
echo "$zero $one\n";
