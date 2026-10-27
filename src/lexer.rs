//! Converts PHP source into tokens.
//! PHP strings are byte strings, so the source is handled as `&[u8]` too.

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    InlineHtml(Vec<u8>),
    Variable(String),
    Ident(String),
    Int(i64),
    Float(f64),
    /// Single-quoted string, or double-quoted string without interpolation
    Str(Vec<u8>),
    /// Double-quoted string with interpolation
    Template(Vec<StrPart>),
    Op(&'static str),
    Semicolon,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(Vec<u8>),
    /// An interpolated expression, e.g. `$a`, `$a[0]` or the inside of `{$a['k']}`
    Code(Vec<Token>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
}

#[derive(Debug)]
pub struct LexError {
    pub msg: String,
    pub line: u32,
}

// Ordered longest first for maximal munch
const OPS: &[&str] = &[
    "<=>", "**=", "===", "!==", "<<=", ">>=", "??=", "?->", "...",
    "==", "!=", "<>", "<=", ">=", "&&", "||", "++", "--", "+=", "-=", "*=", "/=", ".=", "%=",
    "&=", "|=", "^=", "<<", ">>", "**", "??", "->", "=>", "::",
    "+", "-", "*", "/", "%", ".", "=", "<", ">", "!", "?", ":", "(", ")", "{", "}", "[", "]",
    ",", "&", "|", "^", "~", "@", "\\",
];

/// The tokens of a file; `file` names it in compile-time deprecations
pub fn lex(src: &[u8], file: &str) -> Result<Vec<Token>, LexError> {
    Lexer { src, pos: 0, line: 1, out: Vec::new(), file }.run()
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    out: Vec<Token>,
    file: &'a str,
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic() || c >= 0x80
}

fn is_ident_char(c: u8) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.src.get(self.pos + off).copied()
    }

    fn rest(&self) -> &'a [u8] {
        &self.src[self.pos..]
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn push(&mut self, tok: Tok, line: u32) {
        self.out.push(Token { tok, line });
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T, LexError> {
        Err(LexError { msg: msg.into(), line: self.line })
    }

    fn run(mut self) -> Result<Vec<Token>, LexError> {
        loop {
            self.inline_html();
            if self.pos >= self.src.len() {
                break;
            }
            self.php_code()?;
        }
        let line = self.line;
        self.push(Tok::Eof, line);
        Ok(self.out)
    }

    /// Reads everything up to `<?php` as literal output text
    fn inline_html(&mut self) {
        let start = self.pos;
        let line = self.line;
        while self.pos < self.src.len() {
            let r = self.rest();
            if r.len() >= 5 && r[..5].eq_ignore_ascii_case(b"<?php") {
                // `<?php` must be followed by whitespace (or end of file)
                if r.len() == 5 || r[5].is_ascii_whitespace() {
                    break;
                }
            }
            if r.starts_with(b"<?=") {
                break;
            }
            self.bump();
        }
        if self.pos > start {
            let text = self.src[start..self.pos].to_vec();
            self.push(Tok::InlineHtml(text), line);
        }
        let r = self.rest();
        if r.starts_with(b"<?=") {
            self.pos += 3;
            let line = self.line;
            self.push(Tok::Ident("echo".into()), line);
        } else if r.len() >= 5 {
            self.pos += 5;
            // A single whitespace char right after the open tag belongs to the tag
            if self.peek() == Some(b'\r') && self.peek_at(1) == Some(b'\n') {
                self.bump();
            }
            self.bump();
        }
    }

    /// Reads PHP code up to `?>` or EOF
    fn php_code(&mut self) -> Result<(), LexError> {
        loop {
            self.skip_ws_and_comments()?;
            let line = self.line;
            if self.rest().starts_with(b"?>") {
                self.pos += 2;
                // A single newline right after the close tag is not output
                if self.rest().starts_with(b"\r\n") {
                    self.pos += 1;
                }
                if self.peek() == Some(b'\n') {
                    self.bump();
                }
                self.push(Tok::Semicolon, line);
                return Ok(());
            }
            match self.token()? {
                Some(tok) => self.push(tok, line),
                None => return Ok(()),
            }
        }
    }

    /// Lexes one token of PHP code at the current position (whitespace already skipped)
    fn token(&mut self) -> Result<Option<Tok>, LexError> {
        let Some(c) = self.peek() else { return Ok(None) };
        let tok = if c == b'$' && self.peek_at(1).is_some_and(is_ident_start) {
            self.pos += 1;
            Tok::Variable(self.ident())
        } else if c == b'$' && matches!(self.peek_at(1), Some(b'$' | b'{')) {
            // A variable variable: the name follows
            self.pos += 1;
            Tok::Op("$")
        } else if is_ident_start(c) || c == b'\\' && self.peek_at(1).is_some_and(is_ident_start) {
            Tok::Ident(self.qualified_ident())
        } else if c.is_ascii_digit() || (c == b'.' && self.peek_at(1).is_some_and(|d| d.is_ascii_digit())) {
            self.number()?
        } else if c == b'\'' {
            Tok::Str(self.single_quoted()?)
        } else if c == b'"' {
            self.double_quoted()?
        } else if c == b';' {
            self.pos += 1;
            Tok::Semicolon
        } else if let Some(tok) = self.heredoc()? {
            tok
        } else if let Some(op) = OPS.iter().find(|op| self.rest().starts_with(op.as_bytes())) {
            self.pos += op.len();
            Tok::Op(op)
        } else {
            return self.err(format!("syntax error, unexpected character 0x{c:02X}"));
        };
        Ok(Some(tok))
    }

    fn skip_ws_and_comments(&mut self) -> Result<(), LexError> {
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.bump();
                }
                Some(b'#') if self.peek_at(1) != Some(b'[') => self.line_comment(),
                // Attributes are accepted and ignored for now
                Some(b'#') => {
                    let mut depth = 0;
                    while let Some(c) = self.bump() {
                        match c {
                            b'[' => depth += 1,
                            b']' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Some(b'/') if self.peek_at(1) == Some(b'/') => self.line_comment(),
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    self.pos += 2;
                    loop {
                        if self.rest().starts_with(b"*/") {
                            self.pos += 2;
                            break;
                        }
                        if self.bump().is_none() {
                            return self.err("Unterminated comment starting line");
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    /// Line comments end at a newline or `?>`
    fn line_comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == b'\n' || self.rest().starts_with(b"?>") {
                break;
            }
            self.pos += 1;
        }
    }

    /// A name that may contain namespace separators: `Foo`, `\\Foo\\Bar`
    fn qualified_ident(&mut self) -> String {
        let start = self.pos;
        loop {
            if self.peek() == Some(b'\\') && self.peek_at(1).is_some_and(is_ident_start) {
                self.pos += 1;
            }
            if !self.peek().is_some_and(is_ident_start) {
                break;
            }
            self.ident();
            if self.peek() != Some(b'\\') {
                break;
            }
        }
        String::from_utf8_lossy(&self.src[start..self.pos]).into_owned()
    }

    fn ident(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(is_ident_char) {
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.src[start..self.pos]).into_owned()
    }

    fn number(&mut self) -> Result<Tok, LexError> {
        let r = self.rest();
        let radix = match r {
            [b'0', b'x' | b'X', ..] => Some(16),
            [b'0', b'b' | b'B', ..] => Some(2),
            [b'0', b'o' | b'O', ..] => Some(8),
            _ => None,
        };
        if let Some(radix) = radix {
            self.pos += 2;
            let digits = self.digits(|c| (c as char).is_digit(radix));
            return Ok(int_or_float(&digits, radix));
        }

        let int_part = self.digits(|c| c.is_ascii_digit());
        let mut is_float = false;
        let mut text = int_part.clone();
        // PHP's DNUM reads both `1.` and `.5` as floats
        if self.peek() == Some(b'.') {
            self.pos += 1;
            is_float = true;
            text.push('.');
            text.push_str(&self.digits(|c| c.is_ascii_digit()));
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let sign = matches!(self.peek_at(1), Some(b'+' | b'-'));
            let digit_at = if sign { 2 } else { 1 };
            if self.peek_at(digit_at).is_some_and(|c| c.is_ascii_digit()) {
                is_float = true;
                text.push('e');
                if sign {
                    text.push(self.peek_at(1).unwrap() as char);
                }
                self.pos += digit_at;
                text.push_str(&self.digits(|c| c.is_ascii_digit()));
            }
        }
        if is_float {
            return Ok(Tok::Float(text.parse().unwrap_or(0.0)));
        }
        // Integer literals with a leading 0 are octal
        if int_part.len() > 1 && int_part.starts_with('0') {
            if int_part.bytes().any(|c| c >= b'8') {
                return self.err("Invalid numeric literal");
            }
            return Ok(int_or_float(&int_part[1..], 8));
        }
        Ok(int_or_float(&int_part, 10))
    }

    /// Reads a digit run, dropping `_` separators as in `1_000`
    fn digits(&mut self, ok: impl Fn(u8) -> bool) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if ok(c) {
                s.push(c as char);
            } else if c == b'_' && self.peek_at(1).is_some_and(&ok) && !s.is_empty() {
                // Separator, skip it
            } else {
                break;
            }
            self.pos += 1;
        }
        s
    }

    fn single_quoted(&mut self) -> Result<Vec<u8>, LexError> {
        self.bump();
        let mut s = Vec::new();
        loop {
            match self.bump() {
                None => return self.err("syntax error, unterminated string"),
                Some(b'\'') => return Ok(s),
                Some(b'\\') if matches!(self.peek(), Some(b'\'' | b'\\')) => s.push(self.bump().unwrap()),
                Some(c) => s.push(c),
            }
        }
    }

    fn double_quoted(&mut self) -> Result<Tok, LexError> {
        self.bump();
        self.interpolated(Some(b'"'))
    }

    /// The contents of a double-quoted string up to `end`, or of a heredoc body
    /// (the whole input) when `end` is None
    fn interpolated(&mut self, end: Option<u8>) -> Result<Tok, LexError> {
        let mut parts = Vec::new();
        let mut lit = Vec::new();
        loop {
            let Some(c) = self.bump() else {
                if end.is_none() {
                    break;
                }
                return self.err("syntax error, unterminated string");
            };
            match c {
                c if Some(c) == end => break,
                // A heredoc has no quote to escape, so `\"` stays as written
                b'\\' if end.is_none() && self.peek() == Some(b'"') => lit.push(c),
                b'\\' => self.escape(&mut lit),
                b'$' if self.peek().is_some_and(is_ident_start) => {
                    flush(&mut parts, &mut lit);
                    parts.push(StrPart::Code(self.simple_interpolation()?));
                }
                b'{' if self.peek() == Some(b'$') => {
                    flush(&mut parts, &mut lit);
                    parts.push(StrPart::Code(self.complex_interpolation()?));
                }
                b'$' if self.peek() == Some(b'{') => {
                    flush(&mut parts, &mut lit);
                    parts.push(StrPart::Code(self.dollar_brace_interpolation()?));
                }
                _ => lit.push(c),
            }
        }
        if parts.is_empty() {
            return Ok(Tok::Str(lit));
        }
        flush(&mut parts, &mut lit);
        Ok(Tok::Template(parts))
    }

    /// `<<<LABEL`, `<<<"LABEL"` (heredoc) or `<<<'LABEL'` (nowdoc) through the closing
    /// label. None if the input here is not a heredoc header (then it is `<<`)
    fn heredoc(&mut self) -> Result<Option<Tok>, LexError> {
        let src = self.src;
        if !self.rest().starts_with(b"<<<") {
            return Ok(None);
        }
        let mut p = self.pos + 3;
        while matches!(src.get(p), Some(b' ' | b'\t')) {
            p += 1;
        }
        let quote = src.get(p).copied().filter(|&q| q == b'\'' || q == b'"');
        p += usize::from(quote.is_some());
        let label_start = p;
        if !src.get(p).copied().is_some_and(is_ident_start) {
            return Ok(None);
        }
        while src.get(p).copied().is_some_and(is_ident_char) {
            p += 1;
        }
        let label = &src[label_start..p];
        if let Some(q) = quote {
            if src.get(p) != Some(&q) {
                return Ok(None);
            }
            p += 1;
        }
        match &src[p..] {
            [b'\n', ..] => p += 1,
            [b'\r', b'\n', ..] => p += 2,
            _ => return Ok(None),
        }
        let body_start = p;
        // Find the closing label: the first line holding only indentation before it
        let (close_line, indent) = loop {
            let ws = src[p..].iter().take_while(|c| matches!(c, b' ' | b'\t')).count();
            let after = p + ws + label.len();
            if src[p + ws..].starts_with(label) && !src.get(after).copied().is_some_and(is_ident_char) {
                break (p, &src[p..p + ws]);
            }
            match src[p..].iter().position(|&c| c == b'\n') {
                Some(nl) => p += nl + 1,
                None => {
                    self.pos = src.len();
                    self.line += src[body_start..].iter().filter(|&&c| c == b'\n').count() as u32;
                    return self.err("syntax error, unexpected end of file");
                }
            }
        };
        let start_line = self.line;
        // The newline before the closing label is not part of the string
        let body_end = close_line.saturating_sub(1).max(body_start);
        let body_end = if body_end > body_start && src[body_end - 1] == b'\r' { body_end - 1 } else { body_end };
        let mut body = Vec::with_capacity(body_end - body_start);
        if !indent.is_empty() && indent.contains(&b' ') && indent.contains(&b'\t') {
            return self.err("Invalid indentation - tabs and spaces cannot be mixed");
        }
        for (i, line) in src[body_start..body_end].split(|&c| c == b'\n').enumerate() {
            if i > 0 {
                body.push(b'\n');
            }
            let ws = line.iter().take_while(|c| matches!(c, b' ' | b'\t')).count();
            if ws < indent.len() && ws < line.len() {
                self.line = start_line + 1 + i as u32;
                return self.err(format!(
                    "Invalid body indentation level (expecting an indentation level of at least {})",
                    indent.len()
                ));
            }
            body.extend_from_slice(&line[indent.len().min(line.len())..]);
        }
        self.pos = close_line + indent.len() + label.len();
        self.line += src[body_start - 1..self.pos].iter().filter(|&&c| c == b'\n').count() as u32;
        if quote == Some(b'\'') {
            return Ok(Some(Tok::Str(body)));
        }
        let mut sub = Lexer { src: &body, pos: 0, line: start_line + 1, out: Vec::new(), file: self.file };
        Ok(Some(sub.interpolated(None)?))
    }

    /// `"$name"` or `"$name[key]"`, where key is a number, bare word or variable.
    /// The leading `$` has already been consumed
    fn simple_interpolation(&mut self) -> Result<Vec<Token>, LexError> {
        let line = self.line;
        let mut toks = vec![Token { tok: Tok::Variable(self.ident()), line }];
        // `"$obj->prop"` and `"$obj?->prop"` read one property
        for op in ["->", "?->"] {
            if self.rest().starts_with(op.as_bytes()) && self.peek_at(op.len()).is_some_and(is_ident_start) {
                self.pos += op.len();
                toks.push(Token { tok: Tok::Op(if op == "->" { "->" } else { "?->" }), line });
                toks.push(Token { tok: Tok::Ident(self.ident()), line });
                return Ok(toks);
            }
        }
        if self.peek() != Some(b'[') {
            return Ok(toks);
        }
        self.pos += 1;
        let key = match self.peek() {
            Some(b'$') if self.peek_at(1).is_some_and(is_ident_start) => {
                self.pos += 1;
                Tok::Variable(self.ident())
            }
            Some(c) if c.is_ascii_digit() || c == b'-' => {
                let start = self.pos;
                self.pos += 1;
                while self.peek().is_some_and(|d| d.is_ascii_digit()) {
                    self.pos += 1;
                }
                // Kept as a string; the runtime normalizes "1" to int key 1 like PHP does
                Tok::Str(self.src[start..self.pos].to_vec())
            }
            Some(c) if is_ident_start(c) => Tok::Str(self.ident().into_bytes()),
            _ => return self.err("syntax error, unexpected token in string offset"),
        };
        if self.bump() != Some(b']') {
            return self.err("syntax error, unexpected token in string offset, expecting \"]\"");
        }
        toks.extend([Tok::Op("["), key, Tok::Op("]")].map(|tok| Token { tok, line }));
        Ok(toks)
    }

    /// `"{$expr}"`: lexes code tokens up to the matching `}`. The `{` has been consumed
    /// `"${name}"`, `"${name[expr]}"` (a variable) or `"${expr}"` (a variable
    /// variable), both deprecated
    fn dollar_brace_interpolation(&mut self) -> Result<Vec<Token>, LexError> {
        let line = self.line;
        self.bump();
        let name_len = self.rest().iter().take_while(|&&c| is_ident_char(c)).count();
        let simple = self.peek().is_some_and(is_ident_start) && matches!(self.peek_at(name_len), Some(b'}' | b'['));
        if simple {
            crate::diag::compile_deprecated(self.file, line, "Using ${var} in strings is deprecated, use {$var} instead");
            let mut toks = vec![Token { tok: Tok::Variable(self.ident()), line }];
            toks.extend(self.complex_interpolation()?);
            return Ok(toks);
        }
        crate::diag::compile_deprecated(
            self.file,
            line,
            "Using ${expr} (variable variables) in strings is deprecated, use {${expr}} instead",
        );
        let mut toks = vec![Token { tok: Tok::Op("$"), line }, Token { tok: Tok::Op("{"), line }];
        toks.extend(self.complex_interpolation()?);
        toks.push(Token { tok: Tok::Op("}"), line: self.line });
        Ok(toks)
    }

    fn complex_interpolation(&mut self) -> Result<Vec<Token>, LexError> {
        let mut toks = Vec::new();
        let mut depth = 0;
        loop {
            self.skip_ws_and_comments()?;
            let line = self.line;
            let Some(tok) = self.token()? else {
                return self.err("syntax error, unterminated string interpolation");
            };
            match tok {
                Tok::Op("{") => depth += 1,
                Tok::Op("}") if depth == 0 => return Ok(toks),
                Tok::Op("}") => depth -= 1,
                _ => {}
            }
            toks.push(Token { tok, line });
        }
    }

    fn escape(&mut self, out: &mut Vec<u8>) {
        let Some(c) = self.peek() else {
            out.push(b'\\');
            return;
        };
        let simple = match c {
            b'n' => Some(b'\n'),
            b't' => Some(b'\t'),
            b'r' => Some(b'\r'),
            b'v' => Some(0x0b),
            b'e' => Some(0x1b),
            b'f' => Some(0x0c),
            b'\\' => Some(b'\\'),
            b'$' => Some(b'$'),
            b'"' => Some(b'"'),
            _ => None,
        };
        if let Some(b) = simple {
            self.bump();
            out.push(b);
            return;
        }
        if (b'0'..=b'7').contains(&c) {
            let mut v: u32 = 0;
            for _ in 0..3 {
                match self.peek() {
                    Some(d @ b'0'..=b'7') => {
                        v = v * 8 + (d - b'0') as u32;
                        self.bump();
                    }
                    _ => break,
                }
            }
            // Values above 0xFF such as "\400" keep only the low 8 bits
            out.push(v as u8);
            return;
        }
        if c == b'x' && self.peek_at(1).is_some_and(|d| d.is_ascii_hexdigit()) {
            self.bump();
            let mut v: u8 = 0;
            for _ in 0..2 {
                match self.peek() {
                    Some(d) if d.is_ascii_hexdigit() => {
                        v = v * 16 + (d as char).to_digit(16).unwrap() as u8;
                        self.bump();
                    }
                    _ => break,
                }
            }
            out.push(v);
            return;
        }
        if c == b'u' && self.peek_at(1) == Some(b'{') {
            let start = self.pos;
            self.pos += 2;
            let hex = self.digits(|d| d.is_ascii_hexdigit());
            if self.peek() == Some(b'}')
                && let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    self.bump();
                    let mut buf = [0; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    return;
                }
            self.pos = start;
        }
        // Unknown escapes keep the backslash
        out.push(b'\\');
    }
}

fn flush(parts: &mut Vec<StrPart>, lit: &mut Vec<u8>) {
    if !lit.is_empty() {
        parts.push(StrPart::Lit(std::mem::take(lit)));
    }
}

/// Integer literals that overflow i64 become floats
fn int_or_float(digits: &str, radix: u32) -> Tok {
    if digits.is_empty() {
        return Tok::Int(0);
    }
    match i64::from_str_radix(digits, radix) {
        Ok(v) => Tok::Int(v),
        // Let the standard parser handle decimal so rounding is correct
        Err(_) if radix == 10 => Tok::Float(digits.parse().unwrap_or(f64::INFINITY)),
        Err(_) => Tok::Float(digits.chars().fold(0.0, |acc, d| acc * radix as f64 + d.to_digit(radix).unwrap() as f64)),
    }
}
