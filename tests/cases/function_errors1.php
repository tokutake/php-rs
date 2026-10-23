<?php
function add(int $a, int $b = 10): int { return $a + $b; }
echo add(1.5), "\n";
echo add();
