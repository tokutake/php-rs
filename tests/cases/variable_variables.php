<?php
$name = 'foo';
$$name = 1;
echo $foo, "\n";
${'b' . 'ar'} = 2; echo $bar, "\n";
$n2 = 'name'; echo $$n2, " ", $$$n2, "\n";
$arr = ['x' => 5]; $k = 'arr'; echo $$k['x'], " ", ${$k}['x'], "\n";
$$k['y'] = 6; print_r($arr);
function f() { $v = 'local'; $w = 'v'; echo $$w, "\n"; $$w .= '!'; echo $v, "\n"; global $foo; $g = 'foo'; echo $$g, "\n"; }
f();
foreach (['a' => 1, 'b' => 2] as $key => $val) { $$key = $val; }
echo $a + $b, "\n";
var_dump(isset($$name), isset($$k['zz']));
unset($$name); var_dump(isset($foo));
$obj = new stdClass; $p = 'prop'; $obj->$p = 3; $obj->{'pr' . 'op2'} = 4; echo $obj->$p, $obj->{'prop2'}, "\n";
$ref = &$$k; $ref['z'] = 1; echo count($arr), "\n";
echo "${name}\n";
$fn = function() { $q = 'name'; return isset($$q); }; var_dump($fn());
$a = ['k' => 'v']; $n = 'a'; $x = 'n';
echo "${a['k']} ${$x} ${'n'}\n";
echo <<<EOT
heredoc ${n}
EOT;
echo "\n";
