<?php
foreach(['America/New_York','Europe/Berlin','Australia/Sydney'] as $name) {
    $z=new DateTimeZone($name);
    foreach(['2024-03-09 12:00:00','2024-03-30 12:00:00','2024-10-26 12:00:00','2024-11-02 12:00:00','2024-10-05 12:00:00'] as $start) {
        foreach(['P1D','PT24H','P1M'] as $spec) {
            $p=new DatePeriod(new DateTimeImmutable($start,$z),new DateInterval($spec),2);
            echo $name,' ',$start,' ',$spec,':';foreach($p as $d){echo ' ',$d->format('c T U');}echo "\n";
        }
    }
}
$i=new DateInterval('P1D');$i->invert=1;$p=new DatePeriod(new DateTime('2024-01-03'),$i,2);
foreach($p as $d){echo $d->format('Y-m-d'),"\n";}
$i=new DateInterval('PT1S');$i->f=0.25;$p=new DatePeriod(new DateTimeImmutable('2024-01-01 23:59:59.500000'),$i,2);
foreach($p as $d){echo $d->format('Y-m-d H:i:s.u'),"\n";}
$z=new DateTimeZone('Europe/Berlin');
foreach([['2024-10-27 01:30:00','PT1H'],['2024-10-26 02:30:00','P1D']] as [$start,$spec]) {
    foreach(new DatePeriod(new DateTimeImmutable($start,$z),new DateInterval($spec),3) as $d){echo $d->format('c U'),"\n";}
}
