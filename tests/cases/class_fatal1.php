<?php
echo "start\n";
interface Speaks { public function speak(); public function listen(); }
abstract class Animal implements Speaks { abstract public function legs(); }
class Dog extends Animal { public function speak() { return "woof"; } }
