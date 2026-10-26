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
- Named arguments, for user and built-in functions, methods and `new` (also from
  spread arrays and call_user_func_array)
- User functions: type declarations (including intersection and DNF types),
  defaults, variadics, by-reference parameters, `static`, `global`, references
- Exceptions: `try`/`catch`/`finally`, `throw` (also as an expression), the
  built-in Throwable hierarchy, chained previous exceptions
- Closures and arrow functions (`use`, by-reference captures, auto-capture),
  callables, `?->`, `instanceof`
- First-class callable syntax (`strlen(...)`, `$obj->method(...)`, `A::method(...)`,
  `$f(...)`, through `__call`/`__callStatic` too) and `Closure::fromCallable()`
- `Closure::bind()`, `bindTo()` and `call()` with php-src's binding rules, and
  `static` closures
- Classes and interfaces: anonymous classes (including constructor arguments,
  inheritance, interfaces, traits and readonly), inheritance, abstract/final,
  visibility, typed and readonly properties, constructor promotion, static members, constants,
  `self`/`parent`/`static` (late static binding), magic methods (`__get`,
  `__set`, `__call`, `__toString`, `__invoke`, ...), `clone`, user exceptions
- Namespaces: `namespace A;` and braced `namespace A { }`, `use` imports (classes,
  `function`, `const`, aliases, group use), resolution of qualified, fully qualified
  and `namespace\` names, and the global fallback for unqualified functions and constants
- Generators: `yield` (anywhere in an expression), keys, `send()`, `throw()`,
  `getReturn()`, `yield from` arrays, iterators and generators, finally blocks run
  when a suspended generator is destroyed
- `Iterator`, `IteratorAggregate` and `Traversable` in foreach, `iterator_to_array()`,
  `iterator_count()`
- `ArrayAccess` (reads, writes, `isset`/`empty`/`??`, `unset`, compound assignment,
  destructuring, nested objects, and PHP's "Indirect modification" notices) and
  `Countable` for `count()`
- `include` / `require` (and `_once`), with each file's own namespace, strict_types,
  `__FILE__` and locations in diagnostics and traces; `exit` / `die`; `$GLOBALS`,
  `$argv` / `$argc` and the superglobals (`$_SERVER`, `$_ENV`, `$_GET`, ...)
- Destructors: `__destruct` when the last reference goes away, and at the end of
  the script in php-src's order (also after an uncaught exception)
- Traits (`insteadof`/`as` adaptations, static and abstract members, traits using traits)
- Enums: pure and backed, `cases()`/`from()`/`tryFrom()`, methods, constants, interfaces
- Warnings, deprecations and uncaught errors in PHP's format, with line numbers and stack traces
- Built-in functions: type checks and conversions (`is_*`, `intval`, `var_export`, ...),
  strings (`sprintf`/`printf` family, `str_replace`, `str_pad`, `wordwrap`, `strnatcmp`, ...),
  math (`round`, `max`, `number_format`, base conversion, ...), arrays (`array_slice`,
  `array_splice`, `array_column`, `array_unique`, the sort functions with flags,
  `array_walk`, `compact`/`extract`, ...), and the `@` operator
- JSON: `json_encode()` (all the flags, pretty printing, `JsonSerializable`, backed
  enums), `json_decode()` (objects or arrays, big integers, depth limits),
  `json_validate()`, `json_last_error()` and `JSON_THROW_ON_ERROR`; the decoder keeps
  its own stack, so deeply nested input cannot overflow the native one
- Files and streams: resources (`resource(5) of type (stream)`, ids as php-src
  numbers them), `fopen` with plain files, `php://memory`, `php://temp`,
  `php://stdin`/`stdout`/`stderr`/`output` and `STDIN`/`STDOUT`/`STDERR`;
  `fread`/`fgets`/`fgetc`/`fwrite`/`fseek`/`ftell`/`feof` with php-src's
  end-of-file rule, `fprintf`, `fputcsv`/`fgetcsv`/`str_getcsv`,
  `file_get_contents`/`file_put_contents`/`file()`, directories (`opendir`,
  `scandir`, `glob`), `unlink`/`mkdir`/`rename`/`copy`/`touch`/`stat` and friends.
  Reads stop at php-src's default memory limit, so an endless stream cannot
  exhaust memory
- Dates and times: `date`/`gmdate`/`idate`, `time`/`microtime`/`gettimeofday`,
  `strtotime`, `mktime`/`gmmktime`, `checkdate`, `getdate`/`localtime`, default
  timezone functions, and `DateTime`, `DateTimeImmutable`, `DateTimeZone`.
  Formatting, calendar modification, date/time/ISO-week/timestamp/timezone
  setters, offsets, microseconds, cloning, comparisons and the corresponding
  procedural functions are supported. `createFromFormat()` handles numeric and
  textual fields, reset controls, microseconds, timezone and trailing-data flags;
  `getLastErrors()` / `date_get_last_errors()` expose parser warnings and errors.
  `DateInterval` supports ISO durations, relative strings, mutable components,
  formatting and cloning; `DateTime::add()` / `sub()` / `diff()` and their
  procedural forms distinguish calendar days from elapsed hours across DST.
  `strtotime` accepts ISO dates and times,
  Unix timestamps, common English dates and relative day/week/month/year/time
  adjustments. Named zones use `/usr/share/zoneinfo` (IANA TZif), including
  historical transitions and future POSIX `M` rules for daylight saving;
  UTC and fixed offsets work without the database
- Output buffering: `ob_start()` with handlers, chunk sizes and flags, `ob_get_clean()`,
  `ob_end_flush()` and the rest of the ob_* functions. Diagnostics and uncaught errors
  go to the buffers, writes to `STDOUT` bypass them, and buffers left open are
  flushed after the destructors
- Regular expressions: the preg_* functions (`preg_match`, `preg_match_all`,
  `preg_replace`, `preg_replace_callback(_array)`, `preg_split`, `preg_grep`,
  `preg_quote`, ...) on a PCRE2-compatible engine of its own: PCRE2's syntax
  (lookaround, atomic groups, possessive quantifiers, back references, named
  groups, recursion, conditionals, `\K`, backtracking verbs), `/u` with Unicode
  properties, scripts and caseless matching from PCRE2's tables, and PCRE2's
  error messages. Backtracking runs on a heap stack with php-src's limits, so a
  pathological pattern ends in `PREG_BACKTRACK_LIMIT_ERROR` and friends rather
  than a hang or a crash

Not yet: the rest of the date extension (`DatePeriod`, the full timelib
free-form/relative parsing grammar and diagnostics),
network streams and most other extensions. Performance is well behind php-src (roughly 6-10x slower).

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
