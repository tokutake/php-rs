<?php
namespace App\Models;

class User extends Base implements \JsonSerializable
{
    use \App\Greets;

    public function jsonSerialize(): mixed { return ['user' => $this->name]; }
}
