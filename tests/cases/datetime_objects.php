<?php
date_default_timezone_set('UTC');
$a=new DateTime('2024-01-01 00:00:00.000001');
$b=new DateTimeImmutable('2024-01-01 09:00:00.000001',new DateTimeZone('Asia/Tokyo'));
$c=new DateTime('2024-01-01 00:00:00.000002');
var_dump($a==$b,$a===$b,$a<$c,$c>$a,$a<=>$b,$a<=>$c);
var_dump((array)$a,get_object_vars($a),json_encode($a));
foreach ($a as $k=>$v) {echo $k,"\n";}
var_dump(json_encode(new DateTimeZone('Asia/Tokyo')));
var_dump(new DateTimeZone('UTC')==new DateTimeZone('Asia/Tokyo'));
var_dump(new DateTimeZone('UTC')==new DateTimeZone('UTC'));
class MyDate extends DateTime {}
class MyImmutable extends DateTimeImmutable {}
$x=new MyDate('2000-01-01');
$y=new MyImmutable('2000-01-01');
var_dump($x->modify('+1 day') instanceof MyDate,$y->modify('+1 day') instanceof MyImmutable);
echo $x->format('c')," ",$y->format('c'),"\n";
foreach (['UTC','Etc/UTC','GMT','EST','EDT','CET','asia/tokyo'] as $name) {
    $z=new DateTimeZone($name);
    $d=new DateTime('2024-07-01',$z);
    echo $d->format('e T I P'),"\n";
}
