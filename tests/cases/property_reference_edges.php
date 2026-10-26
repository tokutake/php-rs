<?php
function attempt($f){try{var_dump($f());}catch(Throwable $e){echo get_class($e),': ',$e->getMessage(),"\n";}}
class Props {public static int $fresh;public static ?int $nullable;public static int $i=1;public static string $s='a';public int $value=1;public ?int $optional;}
$v='5';Props::$fresh=&$v;var_dump($v,Props::$fresh);$v='6';var_dump($v);
attempt(function(){$r=&Props::$i;$r='bad';});var_dump(Props::$i);
$n=&Props::$nullable;var_dump($n);$n='7';var_dump($n);
$p=new Props;$r=&$p->optional;var_dump($r);$r='8';var_dump($p->optional);
$x=&$p->value;$clone=clone $p;unset($p->value);attempt(function()use(&$x){$x=[];});var_dump($clone->value);
unset($clone);$x=[];var_dump($x);
$source=9;$p->value=&$source;$source='10';var_dump($source,$p->value);unset($p->value);$source=[];var_dump($source);
$a=&Props::$i;attempt(function()use(&$a){Props::$s=&$a;});var_dump(Props::$s,$a);
class NativeOutput {public static string $matches='old';}
attempt(fn()=>preg_match(...['/x/','x'],matches:NativeOutput::$matches));var_dump(NativeOutput::$matches);
