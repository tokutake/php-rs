<?php
namespace App\Models;

echo "loading Base\n";

abstract class Base
{
    public function __construct(public string $name = 'anon') {}
    const KIND = 'model';
}
