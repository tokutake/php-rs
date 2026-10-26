<?php
function attempt($f) { try { var_dump($f()); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; } }
var_dump(date_default_timezone_set('UTC'), date_default_timezone_get());
foreach ([0, -1, 951825296, 1609459200, 1709208000, 2147483648, PHP_INT_MAX, PHP_INT_MIN] as $ts) {
    echo date('Y-m-d H:i:s D l N w z W o S F M n t L y a A g h G u v B e I O P p T Z U c r', $ts), "\n";
}
foreach (['1970-01-01','2000-02-29 12:34:56','2024-02-30','2024-03-01T12:30:45Z','2024-03-01T12:30:45+09:00','@-0.500000','1 January 2000','February 29, 2024','10:20 pm', '+1 day', '+1 month', '-2 weeks', '2 days ago', 'tomorrow', 'yesterday', 'next monday', 'last friday', 'first day of next month', 'last day of this month', '', 'not a date'] as $text) {
    echo $text, ': '; var_dump(strtotime($text, 1709294400));
}
var_dump(checkdate(2,29,2000),checkdate(2,29,1900),checkdate(13,1,2024),checkdate(1,1,0));
var_dump(gmmktime(0,0,0,1,1,70),gmmktime(25,-1,61,2,30,2024));
var_dump(getdate(0),localtime(0),localtime(0,true));
foreach (['d','h','I','L','m','N','o','t','U','W','Y','z','Z','?'] as $f) attempt(fn()=>idate($f,1709208000));
attempt(fn()=>idate('YY',0));
var_dump(date(format:'c',timestamp:0),strtotime(datetime:'2000-01-01',baseTimestamp:0));
var_dump(date_default_timezone_set('Asia/Tokyo'));
echo date('c e T',0), "\n", gmdate('c e T',0), "\n";
var_dump(mktime(0,0,0,1,1,1970));
var_dump(date_default_timezone_set('Invalid/Zone'),date_default_timezone_get());
var_dump(is_int(time()), is_float(microtime(true)), is_string(microtime()), is_array(gettimeofday()), is_float(gettimeofday(true)));
foreach (['Thu, 01 Jan 1970 00:00:00 +0000','2024-00-00','-0500-01-01','70-01-01'] as $text) {
    echo $text, ': '; var_dump(strtotime($text,0));
}
