<?php
namespace Lib\Util {
    const SEP = "-";
    function join(array $a) { return implode(SEP, $a); }
    function shout($s) { return strtoupper($s) . "!"; }
    class Str { static function rev($s) { return strrev($s); } }
    class Box { function __construct(public $v) {} }
    interface Shape {}
    abstract class Base implements Shape { abstract function area(); }
}

namespace Lib\Geo {
    use Lib\Util\Base;
    class Square extends Base {
        function __construct(private $s) {}
        function area() { return $this->s ** 2; }
    }
}

namespace App {
    use Lib\Util\Str;
    use Lib\Util\Box as Container;
    use Lib\Util;
    use Lib\Geo\{Square, Square as Sq};
    use function Lib\Util\join;
    use function Lib\Util\shout as yell;
    use const Lib\Util\SEP;
    use Lib\Util\{Shape, function shout, const SEP as SEPARATOR};

    echo Str::rev("abc"), "\n";
    $b = new Container(5);
    echo get_class($b), " ", $b->v, "\n";
    echo Util\Str::rev("xyz"), "\n";
    echo get_class(new Util\Box(1)), "\n";
    echo join([1, 2, 3]), "\n";
    echo yell("hi"), " ", shout("yo"), "\n";
    echo SEP, SEPARATOR, "\n";
    $s = new Sq(3);
    echo get_class($s), " ", $s->area(), "\n";
    var_dump($s instanceof Shape, $s instanceof Util\Shape, $s instanceof \Lib\Util\Base);
    echo Square::class, " ", Container::class, " ", Util::class, " ", Util\Str::class, "\n";
    function local() { return __FUNCTION__; }
    echo local(), "\n";
    class Thing { function m() { return [__CLASS__, __METHOD__, __FUNCTION__, static::class]; } }
    print_r((new Thing)->m());
    print_r(array_map('App\local', [1]));
    print_r(array_map([Str::class, 'rev'], ['ab']));
    echo implode(",", array_map(fn($x) => yell($x), ['a', 'b'])), "\n";
    try {
        undefined_fn();
    } catch (\Error $e) {
        echo $e->getMessage(), "\n";
    }
    try {
        echo UNDEFINED_CONST;
    } catch (\Error $e) {
        echo $e->getMessage(), "\n";
    }
    try {
        new Missing;
    } catch (\Error $e) {
        echo $e->getMessage(), "\n";
    }
    try {
        Util\nothing();
    } catch (\Error $e) {
        echo $e->getMessage(), "\n";
    }
}

namespace {
    echo __NAMESPACE__ === "" ? "global" : "ns", "\n";
    echo App\local(), "\n";
    echo \Lib\Util\Str::rev("global"), "\n";
    var_dump(new App\Thing);
    function f() { throw new Exception("trace"); }
    function g() { App\h(); }
}

namespace App {
    function h() { \f(); }
    \g();
}
