<?php $items = ['a' => 1, 'b' => 2, 'c' => 3]; ?>
<ul>
<?php foreach ($items as $k => $v): ?>
  <li><?= $k ?>=<?= $v ?><?php if ($v % 2): ?> odd<?php elseif ($v == 2): ?> two<?php else: ?> even<?php endif; ?></li>
<?php endforeach; ?>
</ul>
<?php
$i = 0;
while ($i < 3):
    echo $i++;
endwhile;
echo "\n";
for ($j = 0; $j < 3; $j++):
    if ($j == 1):
        continue;
    endif;
    echo $j;
endfor;
echo "\n";
switch ($i):
    case 3:
        echo "three\n";
        break;
    default:
        echo "other\n";
endswitch;
if (false): echo "no"; else: echo "yes\n"; endif;
if (true):
    foreach ([1, 2] as $n):
        echo $n;
    endforeach;
endif;
echo "\n";
