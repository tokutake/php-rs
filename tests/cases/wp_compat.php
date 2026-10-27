<?php class A { function f() { $a = array(&$this); var_dump($a[0] === $this); $b = [1, &$this]; $f = function(&$x){}; } }
(new A)->f();
#[AllowDynamicProperties]
class B {} $b = new B; $b->x = 1; class C extends B {} $c = new C; $c->y = 2; echo "ok\n";
class J { public function before_request($url, &$headers, &$data, &$type, &$options) { $options['x'] = 1; } }
function dispatch($hook, $parameters = []) { foreach ([[new J, 'before_request']] as $cb) { $cb(...$parameters); } return true; }
$url = 'u'; $headers = []; $data = []; $type = 'GET'; $options = [];
dispatch('h', [&$url, &$headers, &$data, &$type, &$options]);
var_dump($options);
