<?php
date_default_timezone_set('UTC');
foreach ([['!Y-m-d','2024-02-30'],['!Y-m-d','2024-13-40'],['!Y-m-d','bad'],['!Y-m-d','2024-01'],['!Y-m-d','2024-01-01x'],['!Y-m-d+','2024-01-01x'],['!Y-m-d H:i:s','2024-01-01 25:61:61'],['!Y','24'],['!y','69'],['!m','1'],['!u','1'],['!v','1'],['!z','60'],['!Y z','2024 60'],['!d.m.Y','1.2.2024'],['!H:i','1:2'],['!h:i a','12:30 pm'],['!h:i','12:30'],['!a h:i','pm 01:30'],['!Y-m-d D','2024-01-01 Tue'],['!Y-m-d S','2024-01-01 xx'],['!e','Invalid/Zone'],['!U.u','-1.500000'],['!Y-m-d','2024-0-0'],['!c','2024-01-01T00:00:00Z'],['!Y*m*d','2024abc01abc02'],['Y-m-d|','2000-02-29'],['!Y-m-d\TH:i:s.uP','2024-03-01T12:34:56.1234+09:00'],['!F j, Y g:i A','February 29, 2024 3:45 PM'],['!Y#m#d','2024/01/02']] as [$f,$s]) {
    $d=DateTimeImmutable::createFromFormat($f,$s);
    echo json_encode([$f,$s,$d?$d->format('Y-m-d H:i:s.u e'):false,DateTimeImmutable::getLastErrors()]),"\n";
}
class CustomFormat extends DateTimeImmutable {}
$d=CustomFormat::createFromFormat('!Y-m-d','2000-01-01');
var_dump($d instanceof CustomFormat,date_get_last_errors());
var_dump(date_create_from_format(format:'!Y-m-d',datetime:'2000-01-01')->format('c'));
var_dump(date_create_immutable_from_format('!Y','2024',new DateTimeZone('Asia/Tokyo'))->format('c'));
try { DateTime::createFromFormat('Y','20'.chr(0).'24'); } catch(Throwable $e) {echo get_class($e),': ',$e->getMessage(),"\n";}
new DateTime('2024-02-30');var_dump(date_get_last_errors());
new DateTime('2024-01-01');var_dump(date_get_last_errors());
try {new DateTime('bad');}catch(Throwable $e){}var_dump(date_get_last_errors());
foreach ([['!U Y','0 2000'],['!U e','0 Asia/Tokyo'],['!U H','0 12'],['!Y U','2000 0']] as [$f,$s]) {echo DateTime::createFromFormat($f,$s)->format('c U'),"\n";}
