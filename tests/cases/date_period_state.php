<?php
date_default_timezone_set('UTC');
class Mine extends DateTimeImmutable {public $tag='mine';}
class MyInterval extends DateInterval {}
$s=new Mine('2024-01-01');$i=new MyInterval('P1D');$p=new DatePeriod($s,$i,2);
var_dump($p->current,$p->end,isset($p->start),isset($p->current),property_exists($p,'start'));
var_dump($p->start===$p->start,$p->interval===$p->interval,get_class($p->getStartDate()),get_class($p->getDateInterval()),$p->start->tag);
$start=$p->start;$start->tag='changed';$interval=$p->interval;$interval->d=10;$i->d=5;
var_dump($p->start->tag,$p->interval->d);
$it=$p->getIterator();echo get_class($it),"\n";var_dump($p->current);
var_dump($it->key(),$it->valid(),$it->current()===$it->current(),get_class($it->current()),get_class($p->current));
echo $it->current()->format('c'),"\n";
$it->next();echo $it->current()->format('c')," ",$p->current->format('c'),"\n";
$it->rewind();echo $it->current()->format('c'),"\n";
foreach($p as $d){echo get_class($d),' ',$d->format('d'),"\n";}
var_dump($it->valid());
$copy=clone $p;echo json_encode(array_map(fn($d)=>$d->format('d'),iterator_to_array($copy))),"\n";
$vals=get_object_vars($p);var_dump(array_keys($vals));$vals['interval']->d=99;var_dump($p->interval->d);
$nested=[];foreach($p as $a){foreach($p as $b){break;}$nested[]=$a->format('d');}echo json_encode($nested),"\n";
class CustomPeriod extends DatePeriod {}
$custom=new CustomPeriod(new DateTime('2024-01-01'),new DateInterval('P1D'),1);var_dump(iterator_count($custom));
