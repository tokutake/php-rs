<?php
date_default_timezone_set('UTC');
function attempt($f) {try {var_dump($f());}catch(Throwable $e){echo get_class($e), ': ', $e->getMessage(), "\n";}}
attempt(fn()=>new DateTime('bad'));
attempt(fn()=>new DateTimeZone('bad'));
$d=new DateTime('2000-01-01');
attempt(fn()=>$d->modify('bad'));
attempt(fn()=>date_modify($d,'bad'));
attempt(fn()=>timezone_open('bad'));
attempt(fn()=>date_format($d,[]));
attempt(fn()=>date_format($d));
attempt(fn()=>date_date_set($d,[],1,1));
attempt(fn()=>date_time_set($d,1,[]));
attempt(fn()=>date_timezone_set($d,'UTC'));
attempt(fn()=>date_timestamp_get(1));
attempt(fn()=>timezone_offset_get(new DateTimeZone('UTC'),1));
attempt(fn()=>strtotime([]));
attempt(fn()=>checkdate(1,1,[]));
attempt(fn()=>$d->setMicrosecond(-1));
attempt(fn()=>$d->setMicrosecond(1000000));
var_dump(date_default_timezone_set('EDT'),date_default_timezone_set('Z'),date_default_timezone_set('+09:00'));
var_dump(strtotime('foo'.chr(0).'bar',0), date_create('foo'.chr(0).'bar'));
