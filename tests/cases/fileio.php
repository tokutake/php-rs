<?php
// Resources, streams and the file system

echo "-- resources\n";
var_dump(STDIN, STDOUT, STDERR);
$f = fopen('php://memory', 'w+');
var_dump($f, gettype($f), get_debug_type($f), is_resource($f), (int)$f, (string)$f, (bool)$f, $f == $f);
print_r($f); echo "\n"; echo $f, "\n";
var_export($f); echo "\n";
echo json_encode($f), json_last_error_msg(), "\n";
$a = [$f => 1]; var_dump($a);
fclose($f);
var_dump($f, gettype($f), get_debug_type($f), is_resource($f), (int)$f);
$g = fopen(__FILE__, 'r'); var_dump($g);
try { var_dump($g + 1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump($g == 6, $g === $g, $g < 7, intval($g), strval($g), (float)$g, (array)$g);
try { fread($f, 1); } catch (TypeError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { fread("x", 1); } catch (TypeError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }

echo "-- reading and seeking\n";
$p = __DIR__ . '/fileio_tmp.txt';
file_put_contents($p, "ab\ncd\n");
foreach (['file' => $p, 'mem' => 'php://memory'] as $k => $u) {
  echo "== $k\n";
  $f = fopen($u, $k == 'mem' ? 'w+' : 'r');
  if ($k == 'mem') { fwrite($f, "ab\ncd\n"); rewind($f); }
  var_dump(feof($f), fread($f, 6), feof($f), fread($f, 1), feof($f), ftell($f));
  rewind($f);
  while (($l = fgets($f)) !== false) { var_dump($l, feof($f)); }
  var_dump(feof($f), fgets($f), fgetc($f), fread($f, 10), ftell($f));
  rewind($f); var_dump(fgets($f, 2), fgets($f, 3), fgets($f, 1), fgetc($f), ftell($f));
  var_dump(fseek($f, 100), ftell($f), feof($f), fread($f, 1), feof($f), fseek($f, -1), fseek($f, -2, SEEK_END), fread($f, 5), fseek($f, 1, SEEK_CUR), ftell($f));
  var_dump(fwrite($f, "x"));
  var_dump(fstat($f)['size']);
}

$p = __DIR__ . '/fileio_tmp.txt';
file_put_contents($p, "abc");
foreach ([$p, 'php://memory'] as $u) {
$f = fopen($u, 'r+'); if ($u[0] == 'p') { fwrite($f, "abc"); rewind($f); }
var_dump(fread($f, 10), feof($f));
rewind($f); var_dump(fgets($f), feof($f));
rewind($f); var_dump(fgetc($f), fgetc($f), fgetc($f), feof($f), fgetc($f), feof($f));
rewind($f); var_dump(stream_get_contents($f), feof($f));
rewind($f); fread($f, 1); var_dump(fwrite($f, "Z"), ftell($f)); rewind($f); var_dump(fread($f, 10));
var_dump(ftruncate($f, 1), ftell($f), fread($f, 5), feof($f)); rewind($f); var_dump(fread($f, 5));
}
$f = fopen($p, 'a+'); var_dump(ftell($f), fread($f, 10)); fwrite($f, "Q"); var_dump(ftell($f)); rewind($f); var_dump(fread($f, 10));
fseek($f, 0); fwrite($f, "W"); rewind($f); var_dump(fread($f, 10), ftell($f));
$f = fopen($p, 'w'); var_dump(fread($f, 1), file_get_contents($p));
$f = @fopen($p, 'x'); var_dump($f);
$f = fopen('/nonexistent/x', 'r');
var_dump($f);
$f = fopen($p, 'q');
var_dump($f);
$f = fopen(__DIR__, 'r'); var_dump($f, fread($f, 10));
var_dump(fopen('php://memory', 'r'));
unlink($p);

echo "-- file system\n";
$d = __DIR__ . '/fileio_tmp';
@mkdir($d);
var_dump(is_dir($d), mkdir($d), mkdir("$d/a/b"), mkdir("$d/a/b", 0777, true), is_dir("$d/a/b"));
var_dump(file_put_contents("$d/x.txt", "line1\nline2\r\n\nline4"), filesize("$d/x.txt"));
var_dump(file("$d/x.txt"), file("$d/x.txt", FILE_IGNORE_NEW_LINES), file("$d/x.txt", FILE_IGNORE_NEW_LINES | FILE_SKIP_EMPTY_LINES), file("$d/x.txt", FILE_SKIP_EMPTY_LINES));
var_dump(file_put_contents("$d/x.txt", ["a", 1, 2.5], FILE_APPEND), file_get_contents("$d/x.txt"));
var_dump(file_get_contents("$d/x.txt", false, null, 3, 4), file_get_contents("$d/x.txt", false, null, -3));
var_dump(file_put_contents("$d/e.txt", ""), file("$d/e.txt"), file_get_contents("$d/e.txt"));
var_dump(copy("$d/x.txt", "$d/y.txt"), file_get_contents("$d/y.txt"), rename("$d/y.txt", "$d/z.txt"), file_exists("$d/y.txt"), file_exists("$d/z.txt"));
var_dump(scandir($d), scandir($d, SCANDIR_SORT_DESCENDING));
var_dump(glob("$d/*.txt"), glob("$d/*", GLOB_ONLYDIR), glob("$d/*", GLOB_MARK), glob("$d/{x,z}.txt", GLOB_BRACE), glob("$d/nope*"));
var_dump(fnmatch('*.txt', 'a.txt'), fnmatch('[a-c]?', 'bz'), fnmatch('*', '.hidden', FNM_PERIOD), fnmatch('A*', 'abc', FNM_CASEFOLD), fnmatch('a/*', 'a/b/c', FNM_PATHNAME));
$h = opendir($d); $e = []; while (($n = readdir($h)) !== false) $e[] = $n; sort($e); var_dump($e); rewinddir($h); var_dump(readdir($h) !== false); closedir($h);
try { readdir($h); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(is_writable("$d/x.txt"), is_executable("$d/x.txt"), is_link("$d/x.txt"), filetype("$d/x.txt"), filetype($d), fileperms($d) & 0777 ? 'ok' : 'no');
var_dump(touch("$d/t.txt", 1000000000), filemtime("$d/t.txt"), stat("$d/t.txt")['size'], count(stat("$d/t.txt")), chmod("$d/t.txt", 0600), clearstatcache(), fileperms("$d/t.txt") & 0777);
var_dump(unlink("$d/nope"), rmdir("$d/a"), rmdir("$d/nope"), rename("$d/nope", "$d/x"), copy("$d/nope", "$d/x"), filesize("$d/nope"), filemtime("$d/nope"), file_get_contents("$d/nope"), file("$d/nope"), readfile("$d/nope"), opendir("$d/nope"), scandir("$d/nope"));
var_dump(unlink($d), mkdir("$d/x.txt"));
var_dump(readfile("$d/x.txt"));
echo "\n";
foreach (glob("$d/*.txt") as $f) unlink($f);
var_dump(rmdir("$d/a/b"), rmdir("$d/a"), rmdir($d), is_dir($d));
try { file_get_contents(''); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { file_get_contents("a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { file('x', 1024); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(is_dir(sys_get_temp_dir()));
$t = tmpfile(); fwrite($t, "hello"); rewind($t); var_dump(fread($t, 10), stream_get_meta_data($t)['mode']);
$f = fopen('php://memory', 'w+'); fwrite($f, "a,b\n"); var_dump(fputcsv($f, ['x', 'y z', 'q"r', 'a,b', "n\nl", ''], ',', '"', '\\'), fputcsv($f, [1, 2.5, true, null], ';', "'", ''));
rewind($f); var_dump(stream_get_contents($f), stream_get_contents($f, -1, 0), stream_get_contents($f, 3, 1));
var_dump(fprintf($f, "%05d|%s\n", 42, 'x'), vfprintf($f, "%s-%s", ['a', 'b']));
rewind($f); fpassthru($f); echo "\n";
var_dump(stream_get_meta_data($f));
fprintf(STDOUT, "to stdout %d\n", 7); fwrite(STDOUT, "direct\n"); echo "echo\n";
$o = fopen('php://output', 'w'); fwrite($o, "via output\n"); fclose($o);
var_dump(fopen('php://nope', 'r'));
var_dump(get_resource_type($f), get_resource_id($f), is_resource($f)); fclose($f); var_dump(get_resource_type($f), is_resource($f));

echo "-- csv\n";
$lines = ['a,b,c', '"a","b c",d', '"x""y",z', '  "sp"  ,q', 'a,"multi
line",b', '"unterminated', '', ',', 'a,,b', '"a\"b",c', '"a\\"', "a,b\r\n", "\"q\"x,y", 'one', '"",""', ' a , b ', "tab\t,\tx", '"a"b"c"', '"\\\\",x', ';;', 'é,ü'];
foreach ($lines as $l) { var_dump(str_getcsv($l, ',', '"', '\\')); }
var_dump(str_getcsv('a;b;"c;d"', ';', '"', ''), str_getcsv("x|'y|z'", '|', "'", '\\'), str_getcsv('a\"b,c', ',', '"', ''));
$f = fopen('php://memory', 'w+');
fwrite($f, "h1,h2\n1,\"two\nlines\"\n3,4\n\n\"open\nnever\n");
rewind($f);
while (($r = fgetcsv($f, null, ',', '"', '\\')) !== false) var_dump($r);
rewind($f); var_dump(fgetcsv($f, 3, ',', '"', '\\'));
$rows = [['a', 'b c'], ['x"y', "1\n2"], [], ['', ' '], ['\\"q']];
foreach ($rows as $row) { fputcsv($f, $row, ',', '"', '\\'); }
fseek($f, 0); fgets($f);
