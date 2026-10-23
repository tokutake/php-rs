<?php
// PHP 8 comparison rules
var_dump(0 == "a", "1" == "01", "10" == "1e1", 100 == "1e2", "abc" == 0, null == false, null == "", "0" == false, "" == null);
var_dump(1 <=> 2, "a" <=> "b", 2 <=> 1, 1.5 <=> 1.5);
var_dump("abc" < "abd", "10" < "9", "10" < "9a", 10 < "9a");
var_dump(1 === 1.0, "1" === "1", null === null, 1 !== "1");
var_dump(NAN == NAN, NAN < 1, INF > PHP_INT_MAX);
// Type conversions
var_dump((int)"12abc", (int)"  42  ", (float)"1e3", (int)12.9, (int)-12.9, (bool)"0", (bool)"0.0", (string)1.0, (int)"0x1A");
var_dump(0.1 + 0.2, 1e15, 1e17, 123456789012345678.0, 1/3, -2.5, 1e-5, 0.0001);
var_dump(7 <=> "7", "abc" == "ABC", strlen("héllo"));
