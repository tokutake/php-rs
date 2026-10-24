<?php
$name = "World";
$arr = ['k' => 'v', 2 => 'two'];
$obj = new stdClass;
$obj->p = 'prop';
echo <<<EOT
Hello, $name!
  Indented {$arr['k']} and $arr[2] and $obj->p
Escapes: \t tab, \$ dollar, \\ backslash, "quotes" stay
EOT;
echo "\n";
echo <<<"QUOTED"
    Closing marker indentation
      is removed from every line
    $name
    QUOTED;
echo "\n";
echo <<<'NOW'
Nowdoc $name {$arr['k']} \t stays raw
NOW;
echo "\n";
$x = <<<EOT
EOT;
var_dump($x);
$list = [<<<A
  one
  A, <<<B
  two
  B];
var_dump($list);
echo strtoupper(<<<X
  call arg
  X), "\n";
echo <<<END
Line one
END . " concatenated\n";
