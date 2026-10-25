# php-rs

A PHP interpreter written in Rust, aiming for compatibility with PHP 8 while
removing the memory-safety bug classes of the C implementation.

## Goals

- **Safety**: no undefined behavior on integer edge cases (`PHP_INT_MIN % -1`,
  oversized shifts), no crashes on pathological input (deep recursion, deep
  nesting, reference cycles), refused allocations instead of runaway memory use.
- **Compatibility**: output, warnings and error messages match PHP 8.5, checked
  against the reference implementation.

## Status

Early stage. Implemented so far:

- Scalars, strings (byte strings), arrays (ordered hash map, copy-on-write)
- Operators with PHP 8 type juggling and comparison rules
- Control flow: `if`, `while`, `do`, `for`, `foreach` (incl. by reference), `switch`,
  `match`, `break`/`continue N`, and the alternative syntax (`if (...): ... endif;`)
- `declare(strict_types=1)` for user functions, typed properties and built-ins
- Destructuring with `[...]` / `list()` (keyed, nested, by reference, in `foreach`)
- Heredoc and nowdoc strings, `"$obj->prop"` interpolation
- Constants: `const`, `define()`, `constant()`, magic constants (`__LINE__`, `__CLASS__`, ...)
- User functions: type declarations, defaults, variadics, by-reference parameters,
  `static`, `global`, references
- Exceptions: `try`/`catch`/`finally`, `throw` (also as an expression), the
  built-in Throwable hierarchy, chained previous exceptions
- Closures and arrow functions (`use`, by-reference captures, auto-capture),
  callables, `?->`, `instanceof`
- Classes and interfaces: inheritance, abstract/final, visibility, typed and
  readonly properties, constructor promotion, static members, constants,
  `self`/`parent`/`static` (late static binding), magic methods (`__get`,
  `__set`, `__call`, `__toString`, `__invoke`, ...), `clone`, user exceptions
- Destructors: `__destruct` when the last reference goes away, and at the end of
  the script in php-src's order (also after an uncaught exception)
- Traits (`insteadof`/`as` adaptations, static and abstract members, traits using traits)
- Enums: pure and backed, `cases()`/`from()`/`tryFrom()`, methods, constants, interfaces
- Warnings, deprecations and uncaught errors in PHP's format, with line numbers and stack traces
- A small set of built-in functions (`count`, `implode`, `explode`, `usort`, `array_map`, ...)

Not yet: namespaces, generators, most of
the standard library. Performance is well behind php-src (roughly 6-10x slower).

## Usage

```sh
cargo run --release -- script.php
```

## Tests

```sh
cargo test
```

- `tests/cases/*.php` are run and compared with `*.out`, recorded from the
  reference PHP. Re-record them with `tests/regen.sh` (requires `php`).
- `tests/safety.rs` checks that hostile inputs end in a regular PHP error.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
