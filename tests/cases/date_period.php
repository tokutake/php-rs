<?php
date_default_timezone_set('UTC');
function rows($period) {$out=[];foreach($period as $key=>$date){$out[]=[$key,get_class($date),$date->format('Y-m-d H:i:s.u P')];}return $out;}
$start=new DateTime('2024-01-01');$interval=new DateInterval('P1D');
var_dump(DatePeriod::EXCLUDE_START_DATE,DatePeriod::INCLUDE_END_DATE);
foreach([1,2] as $count)foreach([0,1,2,3] as $flags){
    $p=new DatePeriod($start,$interval,$count,$flags);
    echo json_encode([$count,$flags,$p->recurrences,$p->getRecurrences(),$p->include_start_date,$p->include_end_date,rows($p),$p->current?->format('c')]),"\n";
}
foreach(['2024-01-01','2024-01-02','2024-01-03 12:00:00','2023-12-31'] as $end)foreach([0,1,2,3] as $flags){
    $p=new DatePeriod($start,$interval,new DateTimeImmutable($end),$flags);
    echo json_encode([$end,$flags,$p->recurrences,$p->getRecurrences(),rows($p),$p->current?->format('c')]),"\n";
}
$p=new DatePeriod(start:$start,interval:$interval,end:2,options:DatePeriod::EXCLUDE_START_DATE);
var_dump($p instanceof Traversable,$p instanceof IteratorAggregate,iterator_count($p),count(iterator_to_array($p)));
function stream($p){yield from $p;}
echo json_encode(array_map(fn($d)=>$d->format('Y-m-d'),iterator_to_array(stream($p)))),"\n";
foreach([0,1,2,3] as $flags){
    $p=new DatePeriod(new DateTimeImmutable('2024-01-01 00:00:00.500000'),new DateInterval('PT1S'),new DateTimeImmutable('2024-01-01 00:00:02.500000'),$flags);
    echo json_encode(rows($p)),"\n";
}
