<?php
// exit inside a buffer
ob_start(function ($b) { return "[" . $b . "]"; });
echo "a";
ob_start(fn ($b) => strrev($b));
echo "bc";
exit("bye");
