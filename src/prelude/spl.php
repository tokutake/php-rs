<?php
// SPL's iterators and data structures, written in PHP. php-rs loads this file
// the first time one of its classes is needed.

interface OuterIterator extends Iterator { public function getInnerIterator(): ?Iterator; }
interface RecursiveIterator extends Iterator { public function hasChildren(): bool; public function getChildren(): ?RecursiveIterator; }
interface SeekableIterator extends Iterator { public function seek(int $offset): void; }

class ArrayIterator implements SeekableIterator, ArrayAccess, Countable
{
    const STD_PROP_LIST = 1;
    const ARRAY_AS_PROPS = 2;
    private $__storage;
    private $__flags;

    public function __construct(array|object $array = [], int $flags = 0)
    {
        $this->__storage = is_object($array) ? get_object_vars($array) : $array;
        $this->__flags = $flags;
    }
    public function offsetExists(mixed $key): bool { return isset($this->__storage[$key]); }
    public function offsetGet(mixed $key): mixed { return $this->__storage[$key]; }
    public function offsetSet(mixed $key, mixed $value): void { if ($key === null) { $this->__storage[] = $value; } else { $this->__storage[$key] = $value; } }
    public function offsetUnset(mixed $key): void { unset($this->__storage[$key]); }
    public function append(mixed $value): void { $this->__storage[] = $value; }
    public function getArrayCopy(): array { return $this->__storage; }
    public function count(): int { return count($this->__storage); }
    public function getFlags(): int { return $this->__flags; }
    public function setFlags(int $flags): void { $this->__flags = $flags; }
    public function asort(int $flags = SORT_REGULAR): bool { return asort($this->__storage, $flags); }
    public function ksort(int $flags = SORT_REGULAR): bool { return ksort($this->__storage, $flags); }
    public function uasort(callable $callback): bool { return uasort($this->__storage, $callback); }
    public function uksort(callable $callback): bool { return uksort($this->__storage, $callback); }
    public function natsort(): bool { return natsort($this->__storage); }
    public function natcasesort(): bool { return natcasesort($this->__storage); }
    public function rewind(): void { reset($this->__storage); }
    public function current(): mixed { $k = key($this->__storage); return $k === null ? null : $this->__storage[$k]; }
    public function key(): string|int|null { return key($this->__storage); }
    public function next(): void { next($this->__storage); }
    public function valid(): bool { return key($this->__storage) !== null; }
    public function seek(int $offset): void
    {
        if ($offset < 0 || $offset >= count($this->__storage)) {
            throw new OutOfBoundsException("Seek position $offset is out of range");
        }
        reset($this->__storage);
        for ($i = 0; $i < $offset; $i++) next($this->__storage);
    }
    public function __get($name) { return $this->__storage[$name] ?? null; }
    public function __set($name, $value) { $this->__storage[$name] = $value; }
    public function __isset($name) { return isset($this->__storage[$name]); }
    public function __unset($name) { unset($this->__storage[$name]); }
    public function __debugInfo() { return ['storage' => $this->__storage]; }
}

class RecursiveArrayIterator extends ArrayIterator implements RecursiveIterator
{
    const CHILD_ARRAYS_ONLY = 4;
    public function hasChildren(): bool
    {
        $c = $this->current();
        return is_array($c) || (is_object($c) && !($this->getFlags() & self::CHILD_ARRAYS_ONLY));
    }
    public function getChildren(): ?RecursiveArrayIterator { return new static($this->current(), $this->getFlags()); }
}

class ArrayObject implements IteratorAggregate, ArrayAccess, Countable
{
    const STD_PROP_LIST = 1;
    const ARRAY_AS_PROPS = 2;
    private $__storage;
    private $__flags;
    private $__iteratorClass;

    public function __construct(array|object $array = [], int $flags = 0, string $iteratorClass = ArrayIterator::class)
    {
        $this->__storage = is_object($array) ? get_object_vars($array) : $array;
        $this->__flags = $flags;
        $this->__iteratorClass = $iteratorClass;
    }
    public function offsetExists(mixed $key): bool { return isset($this->__storage[$key]); }
    public function offsetGet(mixed $key): mixed { return $this->__storage[$key]; }
    public function offsetSet(mixed $key, mixed $value): void { if ($key === null) { $this->__storage[] = $value; } else { $this->__storage[$key] = $value; } }
    public function offsetUnset(mixed $key): void { unset($this->__storage[$key]); }
    public function append(mixed $value): void { $this->__storage[] = $value; }
    public function getArrayCopy(): array { return $this->__storage; }
    public function exchangeArray(array|object $array): array { $old = $this->__storage; $this->__storage = is_object($array) ? get_object_vars($array) : $array; return $old; }
    public function count(): int { return count($this->__storage); }
    public function getFlags(): int { return $this->__flags; }
    public function setFlags(int $flags): void { $this->__flags = $flags; }
    public function getIterator(): Iterator { $c = $this->__iteratorClass; return new $c($this->__storage); }
    public function getIteratorClass(): string { return $this->__iteratorClass; }
    public function setIteratorClass(string $iteratorClass): void { $this->__iteratorClass = $iteratorClass; }
    public function asort(int $flags = SORT_REGULAR): bool { return asort($this->__storage, $flags); }
    public function ksort(int $flags = SORT_REGULAR): bool { return ksort($this->__storage, $flags); }
    public function uasort(callable $callback): bool { return uasort($this->__storage, $callback); }
    public function uksort(callable $callback): bool { return uksort($this->__storage, $callback); }
    public function natsort(): bool { return natsort($this->__storage); }
    public function natcasesort(): bool { return natcasesort($this->__storage); }
    public function __get($name) { return $this->__storage[$name] ?? null; }
    public function __set($name, $value) { $this->__storage[$name] = $value; }
    public function __isset($name) { return isset($this->__storage[$name]); }
    public function __unset($name) { unset($this->__storage[$name]); }
    public function __debugInfo() { return ['storage' => $this->__storage]; }
}

class IteratorIterator implements OuterIterator
{
    private $__inner;
    private $__valid = false;
    private $__key = null;
    private $__current = null;

    public function __construct(Traversable $iterator, ?string $class = null)
    {
        while ($iterator instanceof IteratorAggregate) {
            $iterator = $iterator->getIterator();
        }
        $this->__inner = $iterator;
    }
    public function getInnerIterator(): ?Iterator { return $this->__inner; }
    protected function __fetch(): void
    {
        $this->__valid = $this->__inner->valid();
        $this->__key = $this->__valid ? $this->__inner->key() : null;
        $this->__current = $this->__valid ? $this->__inner->current() : null;
    }
    public function rewind(): void { $this->__inner->rewind(); $this->__fetch(); }
    public function valid(): bool { return $this->__valid; }
    public function key(): mixed { return $this->__key; }
    public function current(): mixed { return $this->__current; }
    public function next(): void { $this->__inner->next(); $this->__fetch(); }
    public function __call($name, $args) { return $this->__inner->$name(...$args); }
}

abstract class FilterIterator extends IteratorIterator
{
    abstract public function accept(): bool;
    public function __construct(Iterator $iterator) { parent::__construct($iterator); }
    private function __skip(): void
    {
        while (parent::valid() && !$this->accept()) {
            parent::next();
        }
    }
    public function rewind(): void { parent::rewind(); $this->__skip(); }
    public function next(): void { parent::next(); $this->__skip(); }
}

class CallbackFilterIterator extends FilterIterator
{
    private $__callback;
    public function __construct(Iterator $iterator, callable $callback) { parent::__construct($iterator); $this->__callback = $callback; }
    public function accept(): bool { return (bool) ($this->__callback)($this->current(), $this->key(), $this->getInnerIterator()); }
}

abstract class RecursiveFilterIterator extends FilterIterator implements RecursiveIterator
{
    public function __construct(RecursiveIterator $iterator) { parent::__construct($iterator); }
    public function hasChildren(): bool { return $this->getInnerIterator()->hasChildren(); }
    public function getChildren(): ?RecursiveFilterIterator { return new static($this->getInnerIterator()->getChildren()); }
}

class RecursiveCallbackFilterIterator extends CallbackFilterIterator implements RecursiveIterator
{
    private $__cb;
    public function __construct(RecursiveIterator $iterator, callable $callback) { parent::__construct($iterator, $callback); $this->__cb = $callback; }
    public function hasChildren(): bool { return $this->getInnerIterator()->hasChildren(); }
    public function getChildren(): RecursiveCallbackFilterIterator { return new static($this->getInnerIterator()->getChildren(), $this->__cb); }
}

class ParentIterator extends RecursiveFilterIterator
{
    public function accept(): bool { return $this->getInnerIterator()->hasChildren(); }
}

class RecursiveIteratorIterator implements OuterIterator
{
    const LEAVES_ONLY = 0;
    const SELF_FIRST = 1;
    const CHILD_FIRST = 2;
    const CATCH_GET_CHILD = 16;
    private $__root;
    private $__mode;
    private $__flags;
    private $__maxDepth = -1;
    private $__stack = [];
    private $__gen = null;

    public function __construct(Traversable $iterator, int $mode = self::LEAVES_ONLY, int $flags = 0)
    {
        while ($iterator instanceof IteratorAggregate) {
            $iterator = $iterator->getIterator();
        }
        if (!$iterator instanceof RecursiveIterator) {
            throw new InvalidArgumentException('An instance of RecursiveIterator or IteratorAggregate creating it is required');
        }
        $this->__root = $iterator;
        $this->__mode = $mode;
        $this->__flags = $flags;
    }
    private function __walk(RecursiveIterator $it, int $depth)
    {
        $this->__stack[$depth] = $it;
        for ($it->rewind(); $it->valid(); $it->next()) {
            $this->__stack = array_slice($this->__stack, 0, $depth + 1);
            $descend = ($this->__maxDepth < 0 || $depth < $this->__maxDepth) && $this->callHasChildren();
            if (!$descend) {
                yield $it->key() => $it->current();
                continue;
            }
            if ($this->__mode === self::SELF_FIRST) {
                yield $it->key() => $it->current();
            }
            try {
                $children = $this->callGetChildren();
            } catch (Throwable $e) {
                if (!($this->__flags & self::CATCH_GET_CHILD)) throw $e;
                continue;
            }
            $this->beginChildren();
            yield from $this->__walk($children, $depth + 1);
            $this->__stack = array_slice($this->__stack, 0, $depth + 1);
            $this->endChildren();
            if ($this->__mode === self::CHILD_FIRST) {
                yield $it->key() => $it->current();
            }
        }
    }
    public function rewind(): void
    {
        $this->__stack = [];
        $this->__gen = $this->__walk($this->__root, 0);
        $this->__gen->current();
        $this->beginIteration();
    }
    public function valid(): bool { if ($this->__gen === null) return false; $v = $this->__gen->valid(); if (!$v) { $this->endIteration(); } return $v; }
    public function key(): mixed { return $this->__gen?->key(); }
    public function current(): mixed { return $this->__gen?->current(); }
    public function next(): void { $this->__gen?->next(); $this->nextElement(); }
    public function getDepth(): int { return max(0, count($this->__stack) - 1); }
    public function getSubIterator(?int $level = null): ?RecursiveIterator { return $this->__stack[$level ?? $this->getDepth()] ?? null; }
    public function getInnerIterator(): RecursiveIterator { return $this->getSubIterator() ?? $this->__root; }
    public function beginIteration() {}
    public function endIteration() {}
    public function callHasChildren() { return $this->getSubIterator()->hasChildren(); }
    public function callGetChildren() { return $this->getSubIterator()->getChildren(); }
    public function beginChildren() {}
    public function endChildren() {}
    public function nextElement() {}
    public function setMaxDepth(int $maxDepth = -1): void
    {
        if ($maxDepth < -1) throw new OutOfRangeException('RecursiveIteratorIterator::setMaxDepth(): Argument #1 ($maxDepth) must be greater than or equal to -1');
        $this->__maxDepth = $maxDepth;
    }
    public function getMaxDepth(): int|false { return $this->__maxDepth < 0 ? false : $this->__maxDepth; }
    public function __call($name, $args) { return $this->getInnerIterator()->$name(...$args); }
}

class RegexIterator extends FilterIterator
{
    const USE_KEY = 1;
    const INVERT_MATCH = 2;
    const MATCH = 0;
    const GET_MATCH = 1;
    const ALL_MATCHES = 2;
    const SPLIT = 3;
    const REPLACE = 4;
    public ?string $replacement = null;
    private $__regex;
    private $__mode;
    private $__flags;
    private $__pregFlags;
    private $__current;

    public function __construct(Iterator $iterator, string $pattern, int $mode = self::MATCH, int $flags = 0, int $pregFlags = 0)
    {
        parent::__construct($iterator);
        $this->__regex = $pattern;
        $this->__mode = $mode;
        $this->__flags = $flags;
        $this->__pregFlags = $pregFlags;
    }
    public function accept(): bool
    {
        $this->__current = null;
        $subject = (string) ($this->__flags & self::USE_KEY ? parent::key() : parent::current());
        switch ($this->__mode) {
            case self::MATCH:
                $ok = preg_match($this->__regex, $subject) > 0;
                break;
            case self::GET_MATCH:
                $ok = preg_match($this->__regex, $subject, $m, $this->__pregFlags) > 0;
                if ($ok) $this->__current = $m;
                break;
            case self::ALL_MATCHES:
                preg_match_all($this->__regex, $subject, $m, $this->__pregFlags ?: PREG_PATTERN_ORDER);
                $ok = true;
                $this->__current = $m;
                break;
            case self::SPLIT:
                $m = preg_split($this->__regex, $subject, -1, $this->__pregFlags);
                $ok = count($m) > 1;
                if ($ok) $this->__current = $m;
                break;
            default:
                $result = preg_replace($this->__regex, (string) $this->replacement, $subject);
                $ok = $result !== null;
                if ($ok) $this->__current = $result;
        }
        if ($this->__flags & self::INVERT_MATCH) {
            return !$ok;
        }
        return $ok;
    }
    public function current(): mixed { return $this->__current ?? parent::current(); }
    public function getMode(): int { return $this->__mode; }
    public function setMode(int $mode): void { $this->__mode = $mode; }
    public function getFlags(): int { return $this->__flags; }
    public function setFlags(int $flags): void { $this->__flags = $flags; }
    public function getPregFlags(): int { return $this->__pregFlags; }
    public function setPregFlags(int $pregFlags): void { $this->__pregFlags = $pregFlags; }
    public function getRegex(): string { return $this->__regex; }
}

class RecursiveRegexIterator extends RegexIterator implements RecursiveIterator
{
    private $__args;
    public function __construct(RecursiveIterator $iterator, string $pattern, int $mode = self::MATCH, int $flags = 0, int $pregFlags = 0)
    {
        parent::__construct($iterator, $pattern, $mode, $flags, $pregFlags);
        $this->__args = [$pattern, $mode, $flags, $pregFlags];
    }
    public function hasChildren(): bool { return $this->getInnerIterator()->hasChildren(); }
    public function getChildren(): RecursiveRegexIterator { return new static($this->getInnerIterator()->getChildren(), ...$this->__args); }
    public function accept(): bool { return $this->hasChildren() || parent::accept(); }
}

class LimitIterator extends IteratorIterator
{
    private $__offset;
    private $__limit;
    private $__pos = 0;
    public function __construct(Iterator $iterator, int $offset = 0, int $limit = -1)
    {
        if ($offset < 0) throw new ValueError('LimitIterator::__construct(): Argument #2 ($offset) must be greater than or equal to 0');
        if ($limit < -1) throw new ValueError('LimitIterator::__construct(): Argument #3 ($limit) must be greater than or equal to -1');
        parent::__construct($iterator);
        $this->__offset = $offset;
        $this->__limit = $limit;
    }
    public function rewind(): void { parent::rewind(); $this->__pos = 0; while ($this->__pos < $this->__offset && parent::valid()) { parent::next(); $this->__pos++; } }
    public function valid(): bool { return ($this->__limit === -1 || $this->__pos < $this->__offset + $this->__limit) && parent::valid(); }
    public function next(): void { parent::next(); $this->__pos++; }
    public function getPosition(): int { return $this->__pos; }
    public function seek(int $offset): int { $this->rewind(); while ($this->__pos < $offset && parent::valid()) { $this->next(); } return $this->__pos; }
}

class InfiniteIterator extends IteratorIterator
{
    public function next(): void { parent::next(); if (!parent::valid()) { parent::rewind(); } }
}

class NoRewindIterator extends IteratorIterator
{
    private $__started = false;
    public function rewind(): void { if (!$this->__started) { $this->__started = true; $this->__fetch(); } }
    public function valid(): bool { return $this->getInnerIterator()->valid(); }
    public function key(): mixed { return $this->getInnerIterator()->key(); }
    public function current(): mixed { return $this->getInnerIterator()->current(); }
    public function next(): void { $this->getInnerIterator()->next(); }
}

class EmptyIterator implements Iterator
{
    public function current(): never { throw new BadMethodCallException('Accessing the value of an EmptyIterator'); }
    public function key(): never { throw new BadMethodCallException('Accessing the key of an EmptyIterator'); }
    public function next(): void {}
    public function rewind(): void {}
    public function valid(): bool { return false; }
}

class AppendIterator extends IteratorIterator
{
    private $__iterators = [];
    private $__index = 0;
    public function __construct() {}
    public function append(Iterator $iterator): void
    {
        $this->__iterators[] = $iterator;
        if (count($this->__iterators) === 1) { $this->__index = 0; $iterator->rewind(); }
    }
    private function __settle(): void
    {
        while ($this->__index < count($this->__iterators) && !$this->__iterators[$this->__index]->valid()) {
            $this->__index++;
            if ($this->__index < count($this->__iterators)) $this->__iterators[$this->__index]->rewind();
        }
    }
    public function rewind(): void { $this->__index = 0; if ($this->__iterators) { $this->__iterators[0]->rewind(); $this->__settle(); } }
    public function valid(): bool { return $this->__index < count($this->__iterators) && $this->__iterators[$this->__index]->valid(); }
    public function current(): mixed { return $this->valid() ? $this->__iterators[$this->__index]->current() : null; }
    public function key(): mixed { return $this->valid() ? $this->__iterators[$this->__index]->key() : null; }
    public function next(): void { if ($this->valid()) { $this->__iterators[$this->__index]->next(); $this->__settle(); } }
    public function getInnerIterator(): ?Iterator { return $this->__iterators[$this->__index] ?? null; }
    public function getIteratorIndex(): ?int { return $this->valid() ? $this->__index : null; }
    public function getArrayIterator(): ArrayIterator { return new ArrayIterator($this->__iterators); }
}

class SplFileInfo implements Stringable
{
    private $__path;
    public function __construct(string $filename) { $this->__path = $filename; }
    public function getPathname(): string { return $this->__path; }
    public function getFilename(): string { $p = rtrim($this->__path, '/'); $i = strrpos($p, '/'); return $i === false ? $p : substr($p, $i + 1); }
    public function getPath(): string { $p = rtrim($this->__path, '/'); $i = strrpos($p, '/'); return $i === false ? '' : substr($p, 0, $i); }
    public function getBasename(string $suffix = ''): string { return basename($this->__path, $suffix); }
    public function getExtension(): string { $f = $this->getFilename(); $i = strrpos($f, '.'); return $i === false ? '' : substr($f, $i + 1); }
    public function getRealPath(): string|false { return realpath($this->__path); }
    public function isDir(): bool { return is_dir($this->__path); }
    public function isFile(): bool { return is_file($this->__path); }
    public function isLink(): bool { return is_link($this->__path); }
    public function isReadable(): bool { return is_readable($this->__path); }
    public function isWritable(): bool { return is_writable($this->__path); }
    public function getSize(): int|false { return filesize($this->__path); }
    public function getMTime(): int|false { return filemtime($this->__path); }
    public function getType(): string|false { return filetype($this->__path); }
    public function getFileInfo(?string $class = null): SplFileInfo { $c = $class ?? SplFileInfo::class; return new $c($this->__path); }
    public function getPathInfo(?string $class = null): ?SplFileInfo { $c = $class ?? SplFileInfo::class; return new $c($this->getPath()); }
    public function __toString(): string { return $this->__path; }
}

class DirectoryIterator extends SplFileInfo implements SeekableIterator
{
    protected $__dir;
    protected $__entries = [];
    protected $__pos = 0;
    public function __construct(string $directory)
    {
        if ($directory === '') throw new ValueError(static::class . '::__construct(): Argument #1 ($directory) cannot be empty');
        $this->__dir = rtrim($directory, '/');
        $h = @opendir($directory);
        if ($h === false) {
            throw new UnexpectedValueException(static::class . "::__construct($directory): Failed to open directory: No such file or directory");
        }
        while (($e = readdir($h)) !== false) $this->__entries[] = $e;
        closedir($h);
        parent::__construct($this->__entryPath());
    }
    protected function __entryPath(): string { return $this->__dir . '/' . ($this->__entries[$this->__pos] ?? ''); }
    public function isDot(): bool { $e = $this->__entries[$this->__pos] ?? null; return $e === '.' || $e === '..'; }
    public function getFilename(): string { return $this->__entries[$this->__pos] ?? ''; }
    public function getPath(): string { return $this->__dir; }
    public function getPathname(): string { return $this->__entryPath(); }
    public function isDir(): bool { return is_dir($this->__entryPath()); }
    public function isFile(): bool { return is_file($this->__entryPath()); }
    public function isLink(): bool { return is_link($this->__entryPath()); }
    public function getSize(): int|false { return filesize($this->__entryPath()); }
    public function getMTime(): int|false { return filemtime($this->__entryPath()); }
    public function getExtension(): string { $f = $this->getFilename(); $i = strrpos($f, '.'); return $i === false ? '' : substr($f, $i + 1); }
    public function getBasename(string $suffix = ''): string { return basename($this->getFilename(), $suffix); }
    public function getRealPath(): string|false { return realpath($this->__entryPath()); }
    public function rewind(): void { $this->__pos = 0; }
    public function valid(): bool { return $this->__pos < count($this->__entries); }
    public function key(): mixed { return $this->__pos; }
    public function current(): mixed { return $this; }
    public function next(): void { $this->__pos++; }
    public function seek(int $offset): void { $this->__pos = $offset; }
    public function __toString(): string { return $this->getFilename(); }
}

class FilesystemIterator extends DirectoryIterator
{
    const CURRENT_MODE_MASK = 240;
    const CURRENT_AS_PATHNAME = 32;
    const CURRENT_AS_FILEINFO = 0;
    const CURRENT_AS_SELF = 16;
    const KEY_MODE_MASK = 3840;
    const KEY_AS_PATHNAME = 0;
    const FOLLOW_SYMLINKS = 16384;
    const KEY_AS_FILENAME = 256;
    const NEW_CURRENT_AND_KEY = 256;
    const OTHER_MODE_MASK = 28672;
    const SKIP_DOTS = 4096;
    const UNIX_PATHS = 8192;
    protected $__flags;
    public function __construct(string $directory, int $flags = self::KEY_AS_PATHNAME | self::CURRENT_AS_FILEINFO | self::SKIP_DOTS)
    {
        parent::__construct($directory);
        $this->__flags = $flags;
        if ($flags & self::SKIP_DOTS) {
            $this->__entries = array_values(array_filter($this->__entries, fn($e) => $e !== '.' && $e !== '..'));
        }
    }
    public function getFlags(): int { return $this->__flags & ~self::SKIP_DOTS | ($this->__flags & self::SKIP_DOTS); }
    public function setFlags(int $flags): void { $this->__flags = $flags; }
    public function key(): mixed { return $this->__flags & self::KEY_AS_FILENAME ? $this->getFilename() : $this->getPathname(); }
    public function current(): mixed
    {
        if ($this->__flags & self::CURRENT_AS_PATHNAME) return $this->getPathname();
        if ($this->__flags & self::CURRENT_AS_SELF) return $this;
        return new SplFileInfo($this->getPathname());
    }
}

class RecursiveDirectoryIterator extends FilesystemIterator implements RecursiveIterator
{
    private $__subPath = '';
    public function __construct(string $directory, int $flags = self::KEY_AS_PATHNAME | self::CURRENT_AS_FILEINFO)
    {
        parent::__construct($directory, $flags);
    }
    public function hasChildren(bool $allowLinks = false): bool
    {
        if ($this->isDot()) return false;
        $p = $this->getPathname();
        if (!$allowLinks && !($this->__flags & self::FOLLOW_SYMLINKS) && is_link($p)) return false;
        return is_dir($p);
    }
    public function getChildren(): RecursiveDirectoryIterator
    {
        $child = new static($this->getPathname(), $this->__flags);
        $child->__subPath = ($this->__subPath === '' ? '' : $this->__subPath . '/') . $this->getFilename();
        return $child;
    }
    public function getSubPath(): string { return $this->__subPath; }
    public function getSubPathname(): string { return ($this->__subPath === '' ? '' : $this->__subPath . '/') . $this->getFilename(); }
}

class SplObjectStorage implements Countable, Iterator, ArrayAccess
{
    private $__objects = [];
    private $__data = [];
    private $__pos = 0;
    public function attach(object $object, mixed $info = null): void { $h = spl_object_id($object); $this->__objects[$h] = $object; $this->__data[$h] = $info; }
    public function detach(object $object): void { $h = spl_object_id($object); unset($this->__objects[$h], $this->__data[$h]); }
    public function contains(object $object): bool { return isset($this->__objects[spl_object_id($object)]); }
    public function addAll(SplObjectStorage $storage): int { foreach ($storage->__objects as $h => $o) { $this->__objects[$h] = $o; $this->__data[$h] = $storage->__data[$h]; } return count($this->__objects); }
    public function removeAll(SplObjectStorage $storage): int { foreach ($storage->__objects as $h => $o) { unset($this->__objects[$h], $this->__data[$h]); } return count($this->__objects); }
    public function removeAllExcept(SplObjectStorage $storage): int { foreach ($this->__objects as $h => $o) { if (!isset($storage->__objects[$h])) unset($this->__objects[$h], $this->__data[$h]); } return count($this->__objects); }
    public function getInfo(): mixed { $k = array_keys($this->__objects)[$this->__pos] ?? null; return $k === null ? null : $this->__data[$k]; }
    public function setInfo(mixed $info): void { $k = array_keys($this->__objects)[$this->__pos] ?? null; if ($k !== null) $this->__data[$k] = $info; }
    public function getHash(object $object): string { return spl_object_hash($object); }
    public function count(int $mode = COUNT_NORMAL): int { return count($this->__objects); }
    public function rewind(): void { $this->__pos = 0; }
    public function valid(): bool { return $this->__pos < count($this->__objects); }
    public function key(): int { return $this->__pos; }
    public function current(): object { $k = array_keys($this->__objects)[$this->__pos] ?? null; if ($k === null) throw new RuntimeException('Called current() on invalid iterator'); return $this->__objects[$k]; }
    public function next(): void { $this->__pos++; }
    public function offsetExists($object): bool { return $this->contains($object); }
    public function offsetGet($object): mixed { $h = spl_object_id($object); if (!isset($this->__objects[$h])) throw new UnexpectedValueException('Object not found'); return $this->__data[$h]; }
    public function offsetSet($object, mixed $info = null): void { $this->attach($object, $info); }
    public function offsetUnset($object): void { $this->detach($object); }
}

class SplDoublyLinkedList implements Iterator, Countable, ArrayAccess
{
    const IT_MODE_LIFO = 2;
    const IT_MODE_FIFO = 0;
    const IT_MODE_DELETE = 1;
    const IT_MODE_KEEP = 0;
    protected $__items = [];
    protected $__mode = 0;
    protected $__pos = 0;
    public function push(mixed $value): void { $this->__items[] = $value; }
    public function pop(): mixed { if (!$this->__items) throw new RuntimeException("Can't pop from an empty datastructure"); return array_pop($this->__items); }
    public function shift(): mixed { if (!$this->__items) throw new RuntimeException("Can't shift from an empty datastructure"); return array_shift($this->__items); }
    public function unshift(mixed $value): void { array_unshift($this->__items, $value); }
    public function top(): mixed { if (!$this->__items) throw new RuntimeException("Can't peek at an empty datastructure"); return $this->__items[count($this->__items) - 1]; }
    public function bottom(): mixed { if (!$this->__items) throw new RuntimeException("Can't peek at an empty datastructure"); return $this->__items[0]; }
    public function isEmpty(): bool { return !$this->__items; }
    public function count(): int { return count($this->__items); }
    public function toArray(): array { return $this->__items; }
    public function setIteratorMode(int $mode): int { $this->__mode = $mode; return $mode; }
    public function getIteratorMode(): int { return $this->__mode; }
    public function offsetExists($index): bool { return $index >= 0 && $index < count($this->__items); }
    public function offsetGet($index): mixed { if (!$this->offsetExists($index)) throw new OutOfRangeException('SplDoublyLinkedList::offsetGet(): Argument #1 ($index) is out of range'); return $this->__items[$index]; }
    public function offsetSet($index, mixed $value): void { if ($index === null) { $this->__items[] = $value; return; } if (!$this->offsetExists($index)) throw new OutOfRangeException('SplDoublyLinkedList::offsetSet(): Argument #1 ($index) is out of range'); $this->__items[$index] = $value; }
    public function offsetUnset($index): void { if (!$this->offsetExists($index)) throw new OutOfRangeException('SplDoublyLinkedList::offsetUnset(): Argument #1 ($index) is out of range'); array_splice($this->__items, $index, 1); }
    private function __index(): int { return $this->__mode & self::IT_MODE_LIFO ? count($this->__items) - 1 - $this->__pos : $this->__pos; }
    public function rewind(): void { $this->__pos = 0; }
    public function valid(): bool { return $this->__pos >= 0 && $this->__pos < count($this->__items); }
    public function key(): int { return $this->__index(); }
    public function current(): mixed { return $this->valid() ? $this->__items[$this->__index()] : null; }
    public function next(): void
    {
        if ($this->__mode & self::IT_MODE_DELETE) {
            if ($this->__mode & self::IT_MODE_LIFO) array_pop($this->__items); else array_shift($this->__items);
        } else {
            $this->__pos++;
        }
    }
    public function prev(): void { $this->__pos--; }
}

class SplQueue extends SplDoublyLinkedList
{
    public function enqueue(mixed $value): void { $this->push($value); }
    public function dequeue(): mixed { return $this->shift(); }
}

class SplStack extends SplDoublyLinkedList
{
    protected $__mode = 2;
}

class SplFixedArray implements IteratorAggregate, ArrayAccess, Countable, JsonSerializable
{
    private $__items = [];
    private $__size = 0;
    public function __construct(int $size = 0)
    {
        if ($size < 0) throw new ValueError('SplFixedArray::__construct(): Argument #1 ($size) must be greater than or equal to 0');
        $this->__size = $size;
        $this->__items = $size ? array_fill(0, $size, null) : [];
    }
    public static function fromArray(array $array, bool $preserveKeys = true): SplFixedArray
    {
        $size = $preserveKeys && $array ? max(array_keys($array)) + 1 : count($array);
        $f = new SplFixedArray($size);
        $i = 0;
        foreach ($array as $k => $v) { $f[$preserveKeys ? $k : $i++] = $v; }
        return $f;
    }
    private function __check($index, string $fn): int
    {
        if (!is_int($index) && !(is_string($index) && is_numeric($index)) && !is_float($index) && !is_bool($index)) {
            throw new TypeError("Cannot access offset of type " . get_debug_type($index) . " on SplFixedArray");
        }
        $index = (int) $index;
        if ($index < 0 || $index >= $this->__size) throw new RuntimeException('Index invalid or out of range');
        return $index;
    }
    public function offsetExists($index): bool { return is_numeric($index) && $index >= 0 && $index < $this->__size && $this->__items[(int) $index] !== null; }
    public function offsetGet($index): mixed { return $this->__items[$this->__check($index, 'offsetGet')]; }
    public function offsetSet($index, mixed $value): void { if ($index === null) throw new RuntimeException('[] operator not supported for SplFixedArray'); $this->__items[$this->__check($index, 'offsetSet')] = $value; }
    public function offsetUnset($index): void { $this->__items[$this->__check($index, 'offsetUnset')] = null; }
    public function count(): int { return $this->__size; }
    public function getSize(): int { return $this->__size; }
    public function setSize(int $size): bool
    {
        if ($size < $this->__size) { $this->__items = array_slice($this->__items, 0, $size); }
        for ($i = $this->__size; $i < $size; $i++) $this->__items[$i] = null;
        $this->__size = $size;
        return true;
    }
    public function toArray(): array { return $this->__items; }
    public function getIterator(): Iterator { return new ArrayIterator($this->__items); }
    public function jsonSerialize(): array { return $this->__items; }
    public function __wakeup(): void {}
}

function iterator_apply(Traversable $iterator, callable $callback, ?array $args = null): int
{
    $n = 0;
    foreach ($iterator as $_) {
        $n++;
        if (!$callback(...($args ?? []))) break;
    }
    return $n;
}
