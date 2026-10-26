<?php
date_default_timezone_set('UTC');
function attempt($f){try{$value=$f();var_dump($value);}catch(Throwable $e){echo get_class($e),': ',$e->getMessage(),"\n";}}
foreach(['R2/2024-01-01T00:00:00Z/P1D','2024-01-01T00:00:00Z/P1D/2024-01-04T00:00:00Z'] as $spec)foreach([0,1,2,3] as $options){
    $p=DatePeriod::createFromISO8601String(specification:$spec,options:$options);
    echo json_encode([$options,$p->getRecurrences(),array_map(fn($d)=>$d->format('c'),iterator_to_array($p))]),"\n";
}
foreach(['bad','R0/2024-01-01T00:00:00Z/P1D','R/2024-01-01T00:00:00Z/P1D','R2/2024-01-01/P1D','R2/2024-01-01T00:00:00+09:00/P1D','R2/2024-01-01T00:00:00Z/2024-01-04T00:00:00Z','P1D/2024-01-04T00:00:00Z'] as $spec){attempt(fn()=>DatePeriod::createFromISO8601String($spec));}
$p=new DatePeriod('R2/2024-01-01T00:00:00Z/P1D',DatePeriod::EXCLUDE_START_DATE);var_dump(iterator_count($p));
class IsoPeriod extends DatePeriod {}
$p=IsoPeriod::createFromISO8601String('R2/2024-01-01T00:00:00Z/P1D');var_dump($p instanceof IsoPeriod,iterator_count($p));
foreach(['R2/20240101T000000Z/P1D','2024-01-01T00:00:00Z/P1D','R2/2024-01-01T00:00Z/P1D','R2/2024-01-01T00:00:00.5Z/P1D'] as $spec){
    attempt(fn()=>array_map(fn($d)=>$d->format('c'),iterator_to_array(DatePeriod::createFromISO8601String($spec))));
}
