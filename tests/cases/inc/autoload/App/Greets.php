<?php
namespace App;

trait Greets
{
    public function greet(): string { return "hi {$this->name}"; }
}
