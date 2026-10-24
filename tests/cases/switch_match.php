<?php
function kind($v) {
    switch ($v) {
        case 1:
        case 2:
            return "small";
        case "3":
            return "three";
        case null:
            return "nullish";
        default:
            return "other";
    }
}
foreach ([1, 2, 3, "1", 0, null, false, 99] as $v) {
    echo kind($v), "\n";
}

// Fallthrough, break, default in the middle
for ($i = 0; $i < 5; $i++) {
    switch ($i) {
        case 0:
            echo "zero ";
        default:
            echo "default ";
            break;
        case 3:
            echo "three ";
    }
    echo "\n";
}

// break 2 and continue 2 out of a switch inside a loop
foreach ([1, 2, 3, 4] as $n) {
    switch ($n) {
        case 2:
            continue 2;
        case 4:
            break 2;
    }
    echo "n=$n\n";
}

// Alternative syntax and the subject evaluated once
function subject() { echo "eval\n"; return 2; }
switch (subject()) {
    case 1: echo "one\n"; break;
    case 2: echo "two\n"; break;
}

// Empty switch, and case with ;
switch (1) {}
switch (5) {
    case 5;
        echo "semicolon case\n";
}

// match
$x = 3;
echo match ($x) {
    1, 2 => "low",
    3, 4 => "mid",
    default => "high",
}, "\n";
echo match (true) {
    $x < 2 => "a",
    $x < 5 => "b",
    default => "c",
}, "\n";
// Strict comparison
var_dump(match ("1") { 1 => "int", "1" => "string" });
// Arms are evaluated lazily, trailing comma allowed
function arm($s) { echo "arm $s\n"; return $s; }
echo match (2) { arm(1) => "x", arm(2) => "y", arm(3) => "z", }, "\n";
// No match
foreach ([5, "str", 1.5, null, true, [1], new stdClass] as $v) {
    try {
        echo match ($v) { 0 => "zero" };
    } catch (\UnhandledMatchError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
try {
    echo match ("it's a \"quote\"\n") { 0 => "zero" };
} catch (UnhandledMatchError $e) {
    echo $e->getMessage(), "\n";
}
$r = match (1) { 1 => throw new LogicException("thrown from arm"), default => 0 };
