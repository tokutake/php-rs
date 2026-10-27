<?php
class A {}
try { require __DIR__ . "/inc/redeclare.php"; } catch (Error $e) {}
require __DIR__ . "/inc/redeclare.php";
