<?php
date_default_timezone_set('UTC');
foreach ([['2023-01-31','2023-03-01'],['2024-01-31','2024-03-01'],['2023-03-01','2023-01-31'],['2024-03-01','2024-01-31'],['2024-02-29','2025-02-28'],['2025-02-28','2024-02-29'],['2000-01-01 12:30:00.123456','2024-02-03 11:29:59.012345'],['2024-01-01','2024-01-01']] as [$a,$b]) {
    $x=new DateTime($a);$y=new DateTimeImmutable($b);$d=$x->diff($y);
    echo "$a -> $b ",$d->format('%R%y/%m/%d %h:%i:%s.%F %a'),"\n";
    echo $x->diff($y,true)->format('%R%a'),"\n";
}
$z=new DateTimeZone('America/New_York');
foreach ([['2024-03-09 12:00:00','2024-03-10 12:00:00'],['2024-03-10 00:00:00','2024-03-10 04:00:00'],['2024-11-03 00:00:00','2024-11-03 04:00:00'],['2024-03-09 22:00:00','2024-03-10 04:00:00']] as [$a,$b]) {
    $x=new DateTimeImmutable($a,$z);$y=new DateTimeImmutable($b,$z);
    echo "$a -> $b ",$x->diff($y)->format('%R%y/%m/%d %h:%i:%s %a'),"\n";
}
$d=new DateTimeImmutable('2024-03-09 12:00:00',$z);
foreach ([new DateInterval('P1D'),new DateInterval('PT24H'),DateInterval::createFromDateString('24 hours')] as $i) {echo $d->add($i)->format('c'),"\n";}
$x=new DateTimeImmutable('2024-01-01',new DateTimeZone('UTC'));
$y=new DateTimeImmutable('2024-01-01',new DateTimeZone('Asia/Tokyo'));
var_dump(date_diff($x,$y));
class CustomDateInterval extends DateInterval {}
var_dump((new CustomDateInterval('P1D'))->format('%d'));
$a=(new DateTimeImmutable('@1653764124'))->setMicrosecond(137866);
$b=(new DateTimeImmutable('@1699627355'))->setMicrosecond(659663);
$d=$a->diff($b);echo $d->format('%F')," ",$d->f,"\n";
$fold=(new DateTimeImmutable('@1730615400'))->setTimezone(new DateTimeZone('America/New_York'));
foreach (['PT0S','PT1H','P0D'] as $spec) {echo $fold->add(new DateInterval($spec))->format('c U'),"\n";}
