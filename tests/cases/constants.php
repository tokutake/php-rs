<?php
const GREETING = "hello";
const NUMS = [1, 2, 3], TWICE = GREETING . GREETING;
echo GREETING, " ", TWICE, " ", count(NUMS), "\n";
define('DYN', 42);
var_dump(DYN, defined('DYN'), defined('NOPE'), constant('GREETING'));
var_dump(define('DYN', 1));
var_dump(DYN);
echo __LINE__, "\n";
var_dump(str_ends_with(__FILE__, "/constants.php"), str_ends_with(__DIR__, "/cases"));
function fname() { return __FUNCTION__; }
echo fname(), "\n";
class Mc {
    const C = __CLASS__;
    function m() { return __METHOD__ . " " . __FUNCTION__ . " " . __CLASS__; }
    static function s() { return __METHOD__; }
}
echo Mc::C, " ", (new Mc)->m(), " ", Mc::s(), "\n";
var_dump(__CLASS__, __FUNCTION__, __METHOD__);
var_dump(PHP_VERSION_ID >= 80000, PHP_MAJOR_VERSION, M_PI, E_ALL, PHP_OS_FAMILY === 'Linux' || true);
try {
    echo UNDEFINED_THING;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
try {
    constant('ALSO_UNDEFINED');
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
