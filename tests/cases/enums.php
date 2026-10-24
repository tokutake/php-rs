<?php
enum Suit: string {
    case Hearts = 'H';
    case Spades = 'S';
    const Wild = self::Spades;
    public function color(): string { return match($this) { self::Hearts => 'Red', self::Spades => 'Black' }; }
    public static function fromChar(string $c): self { return self::from($c); }
}
enum Status { case Active; case Inactive; }
$o = new stdClass;
var_dump(Suit::Hearts, Status::Active, [Suit::Spades]);
print_r(Suit::Hearts); echo "\n";
print_r(Status::Active); echo "\n";
var_dump(Suit::Hearts->name, Suit::Hearts->value, Suit::Hearts->color(), Suit::Wild);
var_dump(Suit::cases(), Suit::from('S'), Suit::tryFrom('X'), Suit::Hearts === Suit::from('H'), Suit::Hearts == Suit::Spades);
var_dump(Suit::Hearts instanceof UnitEnum, Suit::Hearts instanceof BackedEnum, Status::Active instanceof BackedEnum);
var_dump(new stdClass);
var_dump((array)Suit::Hearts, get_class(Status::Inactive), gettype(Status::Active));
try { Suit::from('X'); } catch (ValueError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $x = Suit::Hearts; $x->name = 'a'; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { new Status(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { clone Status::Active; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump(Status::Active < Status::Inactive, (bool)Status::Active);
try { echo Status::Active; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { Status::from(1); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
enum IntE: int { case One = 1; case Two = 2; }
var_dump(IntE::from('2'), IntE::tryFrom(3));
try { IntE::from('abc'); } catch (TypeError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump(Status::Active->value ?? 'none');
echo "\n";
interface HasLabel { function label(): string; }
trait Shouts { function up() { return strtoupper($this->name); } }
enum Size implements HasLabel {
    use Shouts;
    case Small;
    case Large;
    const DEFAULT = self::Small;
    function label(): string { return ucfirst(strtolower($this->name)); }
    public static function make(): static { return static::DEFAULT; }
}
var_dump(Size::Large->label(), Size::Small instanceof HasLabel, Size::Large->up(), Size::make());
var_dump(Size::Small == Size::Large, Size::Small == Size::Small, Size::Small == true);
var_dump(in_array(Size::Large, [Size::Small]), in_array(Size::Small, [Size::Small], true));
var_dump(enum_exists('Size'), class_exists('Size'), enum_exists('stdClass'));
function sizeName(Size $s): string { return $s->name; }
echo sizeName(Size::Large), "\n";
try { sizeName(Suit::Hearts); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $s = Size::Small; $s->extra = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
$label = match (Size::make()) { Size::Small => "S", Size::Large => "L" };
echo $label, "\n";
var_dump(Size::cases());
