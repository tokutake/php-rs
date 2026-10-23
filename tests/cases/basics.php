<html>
<?php
// echo and variables
$name = "World";
echo "Hello, $name!\n";
echo 'single $name\n', "\n";
echo "braces: {$name}s\n";
$a = 10; $b = 3;
echo $a + $b, " ", $a - $b, " ", $a * $b, " ", $a / $b, " ", $a % $b, " ", $a ** $b, "\n";
echo 7 / 7, " ", 1 + 2 * 3, " ", (1 + 2) * 3, " ", -2 ** 2, " ", 2 ** -1, "\n";
echo "1" + "2", " ", "1.5" + 1, " ", "10" . 5 + 1, "\n";
echo 0.1 + 0.2, " ", 1e100, " ", 1/3, " ", 1e15, " ", 1e14, " ", 1e13, " ", -0.0, "\n";
$s = "a"; $s .= "b"; $s .= 1; echo $s, "\n";
$x = 5; $x += 3; $x -= 1; $x *= 2; echo $x, "\n";
$u = null; $u ??= "default"; echo $u, "\n";
echo true, "|", false, "|", null, "|", 3.0, "\n";
echo "esc: \t|\\|\$|\x41|\101|\u{1F600}|\q\n";
echo 0x1F, " ", 0b101, " ", 0o17, " ", 017, " ", 1_000_000, "\n";
?>
inline <?= "short echo" ?>

<?php
echo "done\n";
