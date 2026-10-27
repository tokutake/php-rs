<?php
$it = new ArrayIterator(['a' => 1, 'b' => 2, 'c' => 3]);
foreach ($it as $k => $v) echo "$k=$v ";
echo count($it), " ", $it['b'], " ", isset($it['z']) ? 'y' : 'n', "\n";
$it['d'] = 4; unset($it['a']); print_r($it->getArrayCopy());
$rai = new RecursiveIteratorIterator(new RecursiveArrayIterator([1, [2, [3, 4]], 5]));
foreach ($rai as $k => $v) echo "$k:$v@", $rai->getDepth(), " "; echo "\n";
foreach (new RecursiveIteratorIterator(new RecursiveArrayIterator(['a' => 1, 'b' => ['c' => 2]]), RecursiveIteratorIterator::SELF_FIRST) as $k => $v) echo $k, is_array($v) ? '[]' : "=$v", " "; echo "\n";
foreach (new RecursiveIteratorIterator(new RecursiveArrayIterator(['a' => 1, 'b' => ['c' => 2]]), RecursiveIteratorIterator::CHILD_FIRST) as $k => $v) echo $k, " "; echo "\n";
$files = [];
foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator(__DIR__ . '/inc/spltest')) as $path => $info) { if (!str_ends_with($path, '.')) $files[] = $path . '|' . get_class($info) . '|' . $info->getFilename(); }
sort($files); print_r($files);
$html = [];
foreach (new RegexIterator(new RecursiveIteratorIterator(new RecursiveDirectoryIterator(__DIR__ . '/inc/spltest')), '/^.+\.html$/i', RecursiveRegexIterator::GET_MATCH) as $m) $html[] = $m[0];
sort($html); print_r($html);
$fi = []; foreach (new FilesystemIterator(__DIR__ . '/inc/spltest') as $k => $f) $fi[] = basename($k); sort($fi); echo implode(',', $fi), "\n";
echo implode(',', iterator_to_array(new CallbackFilterIterator(new ArrayIterator(range(1, 10)), fn($v) => $v % 3 == 0), false)), "\n";
echo implode(',', iterator_to_array(new LimitIterator(new ArrayIterator(range(1, 10)), 2, 3), false)), "\n";
$ao = new ArrayObject(['x' => 1]); $ao['y'] = 2; foreach ($ao as $k => $v) echo "$k$v"; echo count($ao), "\n";
$st = new SplStack; $st->push(1); $st->push(2); foreach ($st as $v) echo $v; echo $st->pop(), "\n";
$q = new SplQueue; $q->enqueue('a'); $q->enqueue('b'); echo $q->dequeue(), count($q), "\n";
$fa = new SplFixedArray(3); $fa[0] = 'x'; echo $fa->getSize(), $fa[0], json_encode($fa->toArray()), "\n";
try { $fa[5] = 1; } catch (RuntimeException $e) { echo $e->getMessage(), "\n"; }
$ii = new IteratorIterator(new ArrayObject([1, 2])); foreach ($ii as $v) echo $v; echo "\n";
var_dump(new ArrayIterator([1]) instanceof Countable, iterator_count(new ArrayIterator([1, 2, 3])));
