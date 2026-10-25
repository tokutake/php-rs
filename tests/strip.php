<?php
// Filter for tests/regen.sh: removes the directory prefix given as the first
// argument from stdin, adjusting the lengths of `string(N) "..."` dumps that
// contain it. Mirrors strip_prefix in tests/compat.rs
$prefix = $argv[1];
$out = stream_get_contents(STDIN);
$res = '';
$i = 0;
$n = strlen($out);
while ($i < $n) {
    if (substr_compare($out, 'string(', $i, 7) === 0
        && preg_match('/\Gstring\((\d+)\) "/', $out, $m, 0, $i)
        && $i + strlen($m[0]) + (int)$m[1] <= $n) {
        $body = str_replace($prefix, '', substr($out, $i + strlen($m[0]), (int)$m[1]));
        $res .= 'string(' . strlen($body) . ') "' . $body;
        $i += strlen($m[0]) + (int)$m[1];
        continue;
    }
    if (substr_compare($out, $prefix, $i, strlen($prefix)) === 0) {
        $i += strlen($prefix);
    } else {
        $res .= $out[$i++];
    }
}
echo $res;
