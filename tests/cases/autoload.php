<?php
// A PSR-4 style loader
spl_autoload_register(function (string $class) {
    echo "autoload($class)\n";
    $file = __DIR__ . '/inc/autoload/' . str_replace('\\', '/', $class) . '.php';
    if (is_file($file)) {
        require $file;
    }
});
$u = new App\Models\User('ann');
echo $u->greet(), " ", json_encode($u), " ", App\Models\User::KIND, "\n";
var_dump($u instanceof App\Models\Base, class_exists('App\Models\User', false));

// Lookups that autoload, and those that do not
var_dump(class_exists('Nope1'), class_exists('Nope2', false), interface_exists('NopeI'), trait_exists('NopeT', false), enum_exists('NopeE'));
var_dump($u instanceof NopeInstance);
try { new Missing(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { Missing2::foo(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { echo Missing3::C; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(is_callable('Missing4::m'), is_callable(['Missing5', 'm']), method_exists('Missing6', 'm'));
var_dump(is_a('Missing7', 'X', true), is_subclass_of('Missing8', 'X'));
var_dump(unserialize('O:8:"Missing9":0:{}'));
echo Missing10::class, "\n";
function typed(?MissingType $x) { return 'typed ok'; }
echo typed(null), "\n";

// Several loaders, prepend, and loaders that define classes themselves
$second = function ($class) {
    echo "second($class)\n";
    if ($class === 'Dyn') {
        class Dyn { const V = 42; }
    }
};
spl_autoload_register($second);
spl_autoload_register(function ($c) { echo "first($c)\n"; }, true, true);
echo Dyn::V, "\n";
var_dump(count(spl_autoload_functions()));
var_dump(spl_autoload_unregister($second), spl_autoload_unregister($second), count(spl_autoload_functions()));

// Re-registering the same callable is ignored; methods can be loaders
class Loader {
    public function load($c) { echo "Loader::load($c)\n"; }
    public static function sload($c) { echo "Loader::sload($c)\n"; }
}
$l = new Loader;
spl_autoload_register([$l, 'load']);
spl_autoload_register([$l, 'load']);
spl_autoload_register('Loader::sload');
spl_autoload_register(['Loader', 'sload']);
var_dump(count(spl_autoload_functions()));
class_exists('Nothing');
spl_autoload_call('ViaCall');

// An exception from a loader propagates
spl_autoload_register(function ($c) { throw new RuntimeException("cannot load $c"); });
try { new Boom(); } catch (RuntimeException $e) { echo $e->getMessage(), "\n"; }

// A loader that asks for the class it is loading does not recurse
foreach (spl_autoload_functions() as $f) spl_autoload_unregister($f);
spl_autoload_register(function ($c) { echo "recursive($c)\n"; var_dump(class_exists($c)); });
var_dump(class_exists('Self1'));
try { spl_autoload_register('no_such_function'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(spl_autoload_functions()[0] instanceof Closure);
// extends triggers the loader
spl_autoload_register(function ($c) { echo "parent($c)\n"; if ($c === 'LateParent') { class LateParent { function who() { return "late"; } } } });
class Child extends LateParent {}
echo (new Child)->who(), "\n";

// The default loader: lowercased names with .inc/.php from the include path
foreach (spl_autoload_functions() as $f) spl_autoload_unregister($f);
chdir(__DIR__ . '/inc/autoload');
spl_autoload_register();
echo spl_autoload_extensions(), "\n";
var_dump(spl_autoload_functions());
echo (new MyLib_Thing)->hi(), "\n";
