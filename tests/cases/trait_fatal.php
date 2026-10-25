<?php
echo "start\n";
trait A { function f() {} }
trait B { function f() {} }
class C { use A, B; }
