<?php
date_default_timezone_set('UTC');
function attempt($f) {try {var_dump($f());}catch(Throwable $e){echo get_class($e),': ',$e->getMessage(),"\n";}}
foreach (['P1Y2M3DT4H5M6S','P1W2D','P0D','PT0S','P0001-02-03T04:05:06','P','PT','P1DT','P1.5D','-P1D'] as $s) {
    echo $s,"\n";attempt(fn()=>new DateInterval($s));
}
$i=new DateInterval('P1Y2M3DT4H5M6S');
echo '[',$i->format('%Y %y %M %m %D %d %H %h %I %i %S %s %F %f %R %r %a %% %q %'),"]\n";
$i->f=0.123456;$i->invert=1;$i->d++;
echo $i->format('%R %r %d %F'),"\n";
var_dump($i->days);$i->days=5;var_dump($i->days,isset($i->y),property_exists($i,'y'));
var_dump((array)$i,get_object_vars($i),json_encode($i));
$copy=clone $i;$copy->y=10;var_dump($i->y,$copy->y);
foreach (['2 days 3 hours','-1 month','2 weeks 3 days','24 hours'] as $s) {
    $j=DateInterval::createFromDateString($s);var_dump((array)$j);
    echo $j->format('%y %m %d %h %i %s %a'),"\n";
    echo (new DateTimeImmutable('2024-01-31'))->add($j)->format('c'),"\n";
}
$d=new DateTime('2024-01-31 12:34:56.987654');
var_dump($d->add(new DateInterval('P1M'))===$d);echo $d->format('Y-m-d H:i:s.u'),"\n";
$d->sub(new DateInterval('P1M'));echo $d->format('Y-m-d H:i:s.u'),"\n";
$i=new DateInterval('PT1S');$i->f=0.123456;$d->add($i);echo $d->format('Y-m-d H:i:s.u'),"\n";
$d=new DateTimeImmutable('2000-01-01');$next=$d->add(new DateInterval('P1D'));var_dump($d!==$next,$d->format('c'),$next->format('c'));
$p=date_create('2000-01-01');date_add($p,new DateInterval('P1D'));date_sub($p,new DateInterval('PT1H'));echo date_format($p,'c'),"\n";
echo date_interval_format(date_interval_create_from_date_string('3 days'),'%d days'),"\n";
attempt(fn()=>date_add($p,1));attempt(fn()=>$p->add(new stdClass));
foreach (['bad','2024-11-23','1 day foo',''] as $s) {
    attempt(fn()=>DateInterval::createFromDateString($s));
    attempt(fn()=>date_interval_create_from_date_string($s));
}
foreach (['tomorrow','yesterday','today','next monday'] as $s) {
    $i=DateInterval::createFromDateString($s);$d=new DateTimeImmutable('2024-01-03 12:34:56');
    echo $s,' ',$i->format('%d %h')," ",$d->add($i)->format('c'),"\n";
    attempt(fn()=>$d->sub($i)->format('c'));
}
