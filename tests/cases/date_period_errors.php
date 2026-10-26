<?php
date_default_timezone_set('UTC');
function attempt($f){try{$value=$f();var_dump($value);}catch(Throwable $e){echo get_class($e),': ',$e->getMessage(),"\n";}}
$s=new DateTime('2000-01-01');$i=new DateInterval('P1D');
foreach([0,-1,PHP_INT_MAX,2147483639,2147483640] as $n){attempt(fn()=>new DatePeriod($s,$i,$n));}
attempt(fn()=>new DatePeriod());attempt(fn()=>new DatePeriod($s,1,2));attempt(fn()=>new DatePeriod($s,$i,'bad'));
$p=new DatePeriod($s,$i,1);
foreach(['start','current','end','interval','recurrences','include_start_date','include_end_date'] as $name){attempt(function()use($p,$name){$p->$name=1;});}
attempt(function()use($p){unset($p->start);});
attempt(function()use($p){$x=&$p->start;});
attempt(function()use($p){foreach($p as &$d){break;}});
attempt(function()use($p){foreach($p->getIterator() as &$d){break;}});
attempt(fn()=>clone $p->getIterator());attempt(fn()=>new InternalIterator());
foreach(['2',true] as $n){attempt(fn()=>(new DatePeriod($s,$i,$n))->getRecurrences());}
