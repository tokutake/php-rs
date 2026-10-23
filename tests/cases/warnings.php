<?php
var_dump((int)1e19, (int)-1e19, (int)NAN, (int)INF);
$s = "a"; $s++; var_dump($s);
$s = "Az"; $s++; var_dump($s);
$s = "zz"; $s++; var_dump($s);
$s = "a9"; $s++; var_dump($s);
$s = "9"; $s++; var_dump($s);
$n = null; $n--; var_dump($n);
