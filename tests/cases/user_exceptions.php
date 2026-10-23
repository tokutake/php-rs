<?php
class AppException extends RuntimeException {
    public function __construct(string $message, private string $context = "none", ?Throwable $previous = null) {
        parent::__construct("[app] $message", 100, $previous);
    }
    public function getContext(): string { return $this->context; }
}
final class NotFound extends AppException {
    public function __toString(): string { return "NotFound: " . $this->getMessage() . " ({$this->getContext()})"; }
}
class ValidationError extends InvalidArgumentException {
    public array $errors = [];
    public static function with(array $errors): static { $e = new static("invalid"); $e->errors = $errors; return $e; }
}
function find($id) { if ($id > 10) throw new NotFound("item $id", "db"); return "item$id"; }
try { find(5); find(99); } catch (AppException $e) {
    var_dump(get_class($e), $e->getMessage(), $e->getCode(), $e->getContext(), $e instanceof RuntimeException, $e->getLine());
    echo $e, "\n";
}
try { throw ValidationError::with(['name' => 'required']); } catch (LogicException $e) { var_dump($e->errors, $e->getMessage()); }
try {
    try { find(50); } catch (NotFound $inner) { throw new AppException("wrapped", "service", $inner); }
} catch (Exception $e) {
    echo get_class($e->getPrevious()), " -> ", $e->getMessage(), "\n";
}
$e = new AppException("dump");
var_dump($e);
function thrower() { throw new NotFound("uncaught", "end"); }
thrower();
