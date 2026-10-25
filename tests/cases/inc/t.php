<?php
if (!function_exists("thrower")) {
    function thrower() { throw new Exception("deep"); }
}
if ($mode == 1) throw new Exception("top of t");
if ($mode == 2) thrower();
if ($mode == 4) strlen([]);
