<?php
foreach (['America/New_York','Europe/London','Europe/Berlin','Australia/Sydney','Asia/Tokyo','+05:30'] as $name) {
    $z = new DateTimeZone($name);
    foreach (['2024-03-10 01:30:00','2024-03-10 02:30:00','2024-03-10 03:30:00','2024-11-03 01:30:00','2024-03-31 02:30:00','2024-10-27 02:30:00','2050-01-01 00:00:00','2050-07-01 00:00:00'] as $s) {
        $d=new DateTime($s,$z);
        echo $name, ' ', $s, ' => ', $d->format('c T I U'), "\n";
    }
}
$z = new DateTimeZone('America/New_York');
$d = new DateTime('2024-03-09 12:00:00',$z);
echo $d->modify('+1 day')->format('c T U'), "\n";
