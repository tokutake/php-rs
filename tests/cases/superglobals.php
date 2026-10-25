<?php
var_dump($argc, $argv);
function f() {
    var_dump(isset($argv), $_SERVER["argc"], $_SERVER["argv"], $_SERVER["PHP_SELF"], $_SERVER["SCRIPT_NAME"], $_SERVER["DOCUMENT_ROOT"]);
    var_dump(is_float($_SERVER["REQUEST_TIME_FLOAT"]), is_int($_SERVER["REQUEST_TIME"]), $_GET, $_POST, $_COOKIE, $_FILES, $_REQUEST);
    var_dump(isset($_ENV["PATH"]), $_SERVER["PATH"] === getenv("PATH"), $_ENV["PATH"] === getenv("PATH"));
    $_SERVER["mine"] = 1;
    $_GET['q'] = 'x';
}
f();
var_dump($_SERVER["mine"], $_GET, array_slice(array_keys($GLOBALS), 0, 7));
var_dump(getenv("PHP_RS_TEST_UNSET"), putenv("PHP_RS_TEST_UNSET=1"), getenv("PHP_RS_TEST_UNSET"), putenv("PHP_RS_TEST_UNSET"), getenv("PHP_RS_TEST_UNSET"));
var_dump(php_sapi_name(), PHP_SAPI, is_array(getenv()), array_slice(array_keys($_SERVER), -8));
try { putenv("=x"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
