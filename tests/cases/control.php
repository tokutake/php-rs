<?php
for ($i = 0; $i < 5; $i++) {
    if ($i == 1) continue;
    if ($i === 4) break;
    echo $i;
}
echo "\n";
$n = 0;
while (true) {
    $n++;
    if ($n > 3) { break; }
    echo "n=$n ";
}
echo "\n";
$i = 10;
do { echo $i--, " "; } while ($i > 7);
echo "\n";
for ($i = 0; $i < 3; $i++) {
    for ($j = 0; $j < 3; $j++) {
        if ($j == 2) continue 2;
        if ($i == 2) break 2;
        echo "$i$j ";
    }
}
echo "\n";
$g = 75;
if ($g >= 90) echo "A";
elseif ($g >= 70) echo "B";
else if ($g >= 50) echo "C";
else echo "D";
echo "\n";
echo $g > 50 ? "pass" : "fail", " ", 0 ?: "elvis", " ", $undefined ?? "coalesce", "\n";
// FizzBuzz
for ($i = 1; $i <= 15; $i++) {
    if ($i % 15 == 0) echo "FizzBuzz";
    elseif ($i % 3 == 0) echo "Fizz";
    elseif ($i % 5 == 0) echo "Buzz";
    else echo $i;
    echo " ";
}
echo "\n";
$t = true and false; var_dump($t);
$t = (true and false); var_dump($t);
var_dump(true xor true, !0, !"0", !"0.0");
