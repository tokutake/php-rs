<?php
// Output buffers are flushed after the destructors, also after an uncaught exception
ob_start(function ($b) { return "[" . $b . "]"; });
echo "before";
class D { function __destruct() { echo "destruct"; } }
$d = new D;
throw new Exception("x");
