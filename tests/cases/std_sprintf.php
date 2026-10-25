<?php
var_dump(sprintf("[%.2x][%5.1x][%-5d][%05d][%+05d][%-05d][%05.1f][%-8.3f][%'*10s][%-10s][%5.2s][%u][%c][%%][%2\$s %1\$s]", 255, 255, 42, 42, 42, 42, 3.14159, 2.5, "hi", "left", "abcdef", -1, 65));
printf("%s is %d years and %.1f%%\n", "Ann", "30", 99.55);
echo sprintf("%b %o %x %X %e %E %.3e %g %G %g %g %g", 10, 64, 255, 255, 12345.678, 0.000123, 1.5, 0.00001234, 1e20, 100000, 1e6, 0.1 + 0.2), "\n";
echo sprintf("%.0f %.0f %.1f %.2f %f %F %.10f %.15f", 0.5, 2.5, 0.25, 1.005, 1/3, -1.5, M_PI, 0.1), "\n";
echo sprintf("%d %d %d %d %s %s %s", "12abc", 3.99, true, null, true, null, 1.0), "\n";
echo sprintf("%5s|%-5s|%05s|%5.1f|%-7.2f|%07.2f|%+.1f|%+d|%+d", "ab", "ab", "ab", -2.25, 1.5, -1.5, 0.0, 0, -3), "\n";
echo sprintf("%1\$s %2\$s %1\$s", "a", "b"), " ", sprintf("%'010d|%-'x10d", 42, 42), "\n";
echo sprintf("%*d|%-*d|%.*f", 5, 42, 4, 7, 2, M_PI), "\n";
echo vsprintf("%04d-%02d-%02d", [2024, 3, 7]), " ", vsprintf("%s=%s", ['k' => 'a', 'v' => 'b']), "\n";
echo sprintf("%e %.0e %E", 0, 1234.5, -0.000012345), "\n";
echo sprintf("%g %g %g %G %.3g %.10g", 0.0001234, 123456789, -1.5e-10, 1e-5, 3.14159, 1/3), "\n";
echo sprintf("%f %e %g", INF, -INF, NAN), "|", sprintf("%5.1f|%-6s|", NAN, INF), "\n";
echo sprintf("%s", [1]), "\n";
echo sprintf("%.60f", 1), "\n";
var_dump(vprintf("%d-%d\n", [1, 2]));
foreach (['%', '%y', '%1$s %3$s'] as $f) {
    try { echo sprintf($f, 'a'), "\n"; } catch (ValueError | ArgumentCountError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
try { sprintf("%d %d", 1); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
try { vsprintf("%d %d", [1]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { sprintf("%0\$s", 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
