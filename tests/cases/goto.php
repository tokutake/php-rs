<?php
// Forward and backward jumps, out of loops and between switch cases
function scan(array $xs) {
    $out = [];
    $i = 0;
    loop:
    if ($i >= count($xs)) goto done;
    foreach ([1, 2, 3] as $k) {
        if ($xs[$i] === $k) {
            $out[] = "hit $k";
            $i++;
            goto loop;
        }
    }
    $out[] = "miss {$xs[$i]}";
    $i++;
    goto loop;
    done:
    return $out;
}
print_r(scan([1, 5, 3]));
function sw($op) {
    switch ($op) {
        case 'a':
            echo "a\n";
            goto shared;
        case 'b':
            echo "b\n";
            shared:
            echo "shared\n";
            break;
        case 'c':
            $n = 0;
            again:
            $n++;
            if ($n < 3) goto again;
            echo "c $n\n";
    }
    goto end;
    echo "skipped\n";
    end:
    echo "end $op\n";
}
sw('a'); sw('b'); sw('c');
function gen() {
    $i = 0;
    top:
    yield $i;
    if (++$i < 3) goto top;
}
foreach (gen() as $v) echo "gen $v\n";
try {
    goto after;
    echo "not here\n";
} finally {
    echo "finally\n";
}
after:
echo "after\n";
$x = 0;
retry:
while (true) {
    $x++;
    if ($x < 3) goto retry;
    break;
}
echo "x=$x\n";
