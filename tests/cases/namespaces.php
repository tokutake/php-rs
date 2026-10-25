<?php
namespace App\Models;

const VERSION = "1.0";
define('App\Models\BUILD', 42);

interface HasName { function name(): string; }

trait Greets {
    function greet() { return "Hello from " . __CLASS__ . " / " . __TRAIT__ . " / " . __METHOD__; }
}

class User implements HasName {
    use Greets;
    function __construct(private string $n) {}
    function name(): string { return $this->n; }
    static function make(string $n): static { return new static($n); }
}

enum Role: string { case Admin = 'admin'; case Guest = 'guest'; }

function helper($x) { return __FUNCTION__ . "($x)"; }

function strlen($s) { return "my strlen"; }

echo __NAMESPACE__, "\n";
echo User::class, "\n";
echo helper(1), "\n";
echo strlen("abc"), "\n";
echo \strlen("abc"), "\n";
echo strtoupper("fallback"), "\n";
echo VERSION, " ", BUILD, " ", \App\Models\VERSION, " ", namespace\VERSION, "\n";
echo PHP_INT_SIZE, " ", \PHP_INT_SIZE, " ", E_ALL, "\n";
$u = new User("ann");
echo get_class($u), " ", $u->name(), "\n";
echo $u->greet(), "\n";
var_dump($u instanceof HasName, $u instanceof \App\Models\HasName, $u instanceof namespace\User);
echo Role::Admin->value, " ", get_class(Role::Guest), "\n";
var_dump(Role::from('guest'));
echo get_class(User::make("bob")), "\n";
var_dump(class_exists('App\Models\User'), class_exists('\App\Models\User'), class_exists('User'));
var_dump(function_exists('App\Models\helper'), function_exists('helper'));
echo call_user_func('App\Models\helper', 2), "\n";
echo call_user_func(__NAMESPACE__ . '\helper', 3), "\n";
$cls = 'App\Models\User';
echo (new $cls("dyn"))->name(), "\n";
var_dump(defined('App\Models\VERSION'), constant('App\Models\BUILD'), defined('\App\Models\VERSION'));
echo \App\Models\helper(4), "\n";
$f = fn($x) => $x * 2;
echo $f(21), "\n";
try {
    throw new \RuntimeException("rt");
} catch (\RuntimeException $e) {
    echo get_class($e), " ", $e->getMessage(), "\n";
}
try {
    throw new \InvalidArgumentException("ia");
} catch (\LogicException $e) {
    echo "caught ", get_class($e), "\n";
}
class MyException extends \Exception {}
try { throw new MyException("mine"); } catch (MyException $e) { echo get_class($e), "\n"; }
function typed(User $u, ?HasName $h = null): HasName { return $u; }
echo get_class(typed($u)), "\n";
try { typed("x"); } catch (\TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump($u);
print_r(Role::cases());
