<?php
function badret(): int { return "abc"; }
function wrap() { return badret(); }
echo wrap();
