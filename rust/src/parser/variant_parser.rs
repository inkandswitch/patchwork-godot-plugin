use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use indexmap::IndexMap;

use base64::Engine;
use lexical::{NumberFormatBuilder, WriteFloatOptions};
use num_traits::{Float};

use crate::parser::parser_defs::{ElemType, RealT, VariantVal};

const REALT_IS_DOUBLE: bool = false;


// -----------------------------------------------------------------------------
// Parse error
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct VariantParseError(pub String);

impl Display for VariantParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VariantParseError {}


// -----------------------------------------------------------------------------
// Float formatting (Godot rtos_fix / num_scientific via lexical Grisu2)
// -----------------------------------------------------------------------------

const LEXICAL_FORMAT: u128 = NumberFormatBuilder::new()
    .required_exponent_sign(true)
    .build_strict();

/// Trait to call lexical's formatter with the correct type (f32 vs f64). Rust cannot
/// compare generic `T` to a type like `f32`, so we dispatch via this trait instead.
trait FormatFloatLexical {
    fn format_float_lexical(self, options: &WriteFloatOptions) -> String;
}
impl FormatFloatLexical for f32 {
    fn format_float_lexical(self, options: &WriteFloatOptions) -> String {
        lexical::to_string_with_options::<f32, LEXICAL_FORMAT>(self, options)
    }
}
impl FormatFloatLexical for f64 {
    fn format_float_lexical(self, options: &WriteFloatOptions) -> String {
        lexical::to_string_with_options::<f64, LEXICAL_FORMAT>(self, options)
    }
}

/// Formats a float for variant text output. Matches Godot's rtos_fix/num_scientific:
/// 0 → "0", nan → "nan", inf → "inf", -inf → "-inf", finite → Grisu2 short decimal.
pub fn format_float_for_variant<T: Float + FormatFloatLexical>(value: T) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() { "-inf" } else { "inf" }.to_string();
    }
    let options = WriteFloatOptions::builder()
        .trim_floats(true)
        .build()
        .unwrap();
    value.format_float_lexical(&options)
}

/// Float for variant output: use format_float_for_variant then append ".0" if result looks like an integer (Godot rule).
fn rtos_fix_impl<T: Float + FormatFloatLexical>(value: T, compat: bool) -> String {
    if value.is_zero() {
        return "0".to_string();
    }
    if compat && value.is_infinite() && value.is_sign_negative() {
        return "inf_neg".to_string();
    }
    let s = format_float_for_variant(value);
    if s == "inf" || s == "-inf" || s == "nan" {
        return s;
    }
    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
        format!("{}.0", s)
    } else {
        s
    }
}
trait RTosFix {
    fn rtos_fix(&self, compat: bool) -> String;
}

impl RTosFix for f32 {
    fn rtos_fix(&self, compat: bool) -> String {
        rtos_fix_impl(*self, compat)
    }
}

impl RTosFix for f64 {
    fn rtos_fix(&self, compat: bool) -> String {
        rtos_fix_impl(*self, compat)
    }
}


// -----------------------------------------------------------------------------
// RealT (f32/f64 for Godot real_t)
// -----------------------------------------------------------------------------


impl RTosFix for RealT {
    fn rtos_fix(&self, compat: bool) -> String {
        match self {
            RealT::F32(f) => rtos_fix_impl(*f, compat),
            RealT::F64(f) => rtos_fix_impl(*f, compat),
        }
    }
}


/// Escape string for variant text (multiline style: only \ and ").
fn escape_string_for_variant(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}
// -----------------------------------------------------------------------------
// Lexer (tokenizer) — mirrors Godot's get_token
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Token {
    CurlyOpen,
    CurlyClose,
    BracketOpen,
    BracketClose,
    ParenOpen,
    ParenClose,
    Colon,
    Comma,
    Identifier(String),
    Str(String),
    StringName(String),
    Number { int: Option<i64>, float: f64 }, // if int is Some, value is integer; else float
    Color { r: f32, g: f32, b: f32, a: f32 },
    Eof,
}

fn is_underscore(c: char) -> bool {
    c == '_'
}

fn stor_fix(s: &str) -> Option<f64> {
    match s {
        "inf" => Some(f64::INFINITY),
        "-inf" | "inf_neg" => Some(f64::NEG_INFINITY),
        "nan" => Some(f64::NAN),
        _ => None,
    }
}

/// Parse #hex color to (r, g, b, a) in 0.0..=1.0. Matches Godot Color::html().
/// Supports: #rgb (3), #rgba (4), #rrggbb (6), #rrggbbaa (8).
fn parse_color_hex(hex: &str) -> Result<(f32, f32, f32, f32), VariantParseError> {
    fn parse_col4(s: &str, ofs: usize) -> Result<f32, VariantParseError> {
        let c = s.chars().nth(ofs).ok_or_else(|| VariantParseError("Invalid color code".into()))?;
        let v = match c {
            '0'..='9' => c as u32 - '0' as u32,
            'a'..='f' => c as u32 - 'a' as u32 + 10,
            'A'..='F' => c as u32 - 'A' as u32 + 10,
            _ => return Err(VariantParseError("Invalid color code".into())),
        };
        Ok(v as f32)
    }
    fn parse_col8(s: &str, ofs: usize) -> Result<f32, VariantParseError> {
        let hi = parse_col4(s, ofs)? as u32;
        let lo = parse_col4(s, ofs + 1)? as u32;
        Ok((hi * 16 + lo) as f32)
    }

    let n = hex.len();
    let (r, g, b, a) = if n == 3 {
        let r = parse_col4(hex, 0)? / 15.0;
        let g = parse_col4(hex, 1)? / 15.0;
        let b = parse_col4(hex, 2)? / 15.0;
        (r, g, b, 1.0)
    } else if n == 4 {
        let r = parse_col4(hex, 0)? / 15.0;
        let g = parse_col4(hex, 1)? / 15.0;
        let b = parse_col4(hex, 2)? / 15.0;
        let a = parse_col4(hex, 3)? / 15.0;
        (r, g, b, a)
    } else if n == 6 {
        let r = parse_col8(hex, 0)? / 255.0;
        let g = parse_col8(hex, 2)? / 255.0;
        let b = parse_col8(hex, 4)? / 255.0;
        (r, g, b, 1.0)
    } else if n == 8 {
        let r = parse_col8(hex, 0)? / 255.0;
        let g = parse_col8(hex, 2)? / 255.0;
        let b = parse_col8(hex, 4)? / 255.0;
        let a = parse_col8(hex, 6)? / 255.0;
        (r, g, b, a)
    } else {
        return Err(VariantParseError(format!("Invalid color code: expected 3, 4, 6, or 8 hex digits, got {}", n)));
    };
    if r < 0.0 || g < 0.0 || b < 0.0 || a < 0.0 {
        return Err(VariantParseError("Invalid color code".into()));
    }
    Ok((r, g, b, a))
}

struct Lexer<'a> {
    chars: std::str::Chars<'a>,
    saved: Option<char>,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a str) -> Self {
        Lexer {
            chars: s.chars(),
            saved: None,
        }
    }

    fn get_char(&mut self) -> Option<char> {
        if let Some(c) = self.saved.take() {
            return Some(c);
        }
        self.chars.next()
    }

    fn put_back(&mut self, c: char) {
        debug_assert!(self.saved.is_none());
        self.saved = Some(c);
    }

    fn next_token(&mut self) -> Result<Token, VariantParseError> {
        loop {
            let cchar = self.get_char();
            let cchar = match cchar {
                None => return Ok(Token::Eof),
                Some('\n') => continue,
                Some(c) if c <= ' ' => continue,
                Some(c) => c,
            };

            return match cchar {
                '{' => Ok(Token::CurlyOpen),
                '}' => Ok(Token::CurlyClose),
                '[' => Ok(Token::BracketOpen),
                ']' => Ok(Token::BracketClose),
                '(' => Ok(Token::ParenOpen),
                ')' => Ok(Token::ParenClose),
                ':' => Ok(Token::Colon),
                ',' => Ok(Token::Comma),
                ';' => {
                    while let Some(ch) = self.get_char() {
                        if ch == '\n' {
                            break;
                        }
                    }
                    continue;
                }
                '#' => {
                    let mut hex = String::new();
                    loop {
                        match self.get_char() {
                            Some(ch) if ch.is_ascii_hexdigit() => hex.push(ch),
                            other => {
                                if let Some(c) = other {
                                    self.put_back(c);
                                }
                                break;
                            }
                        }
                    }
                    // Match Godot Color::html(): #rgb (3), #rgba (4), #rrggbb (6), #rrggbbaa (8)
                    let (r, g, b, a) = parse_color_hex(&hex)?;
                    Ok(Token::Color { r, g, b, a })
                }
                '&' => {
                    if self.get_char() != Some('"') {
                        return Err(VariantParseError("Expected '\"' after '&'".into()));
                    }
                    let s = self.parse_string()?;
                    return Ok(Token::StringName(s));
                }
                '"' => {
                    let s = self.parse_string()?;
                    Ok(Token::Str(s))
                }
                '-' => {
                    let next = self.get_char();
                    match next {
                        Some(c) if c.is_ascii_digit() => {
                            self.put_back(c);
                            self.put_back('-');
                            Ok(self.parse_number()?)
                        }
                        Some(c) if c.is_ascii_alphabetic() || is_underscore(c) => {
                            // Identifier like -inf, inf_neg (Godot allows minus-prefix for these)
                            let mut token_text = String::from("-");
                            let mut cur = c;
                            let mut first = true;
                            loop {
                                if cur.is_ascii_alphabetic() || is_underscore(cur) || (!first && cur.is_ascii_digit()) {
                                    token_text.push(cur);
                                    first = false;
                                    cur = match self.get_char() {
                                        Some(c) => c,
                                        None => break,
                                    };
                                } else {
                                    self.put_back(cur);
                                    break;
                                }
                            }
                            Ok(Token::Identifier(token_text))
                        }
                        other => {
                            if let Some(c) = other {
                                self.put_back(c);
                            }
                            self.put_back('-');
                            Err(VariantParseError("Unexpected character '-'".into()))
                        }
                    }
                }
                c if c.is_ascii_digit() => {
                    self.put_back(c);
                    Ok(self.parse_number()?)
                }
                c if c.is_ascii_alphabetic() || is_underscore(c) => {
                    let mut token_text = String::new();
                    let mut cur = c;
                    let mut first = true;
                    loop {
                        if cur.is_ascii_alphabetic() || is_underscore(cur) || (!first && cur.is_ascii_digit()) {
                            token_text.push(cur);
                            first = false;
                            cur = match self.get_char() {
                                Some(c) => c,
                                None => break,
                            };
                        } else {
                            self.put_back(cur);
                            break;
                        }
                    }
                    Ok(Token::Identifier(token_text))
                }
                _ => Err(VariantParseError(format!("Unexpected character '{}'", cchar))),
            };
        }
    }

    fn parse_string(&mut self) -> Result<String, VariantParseError> {
        let mut s = String::new();
        loop {
            let ch = self.get_char().ok_or_else(|| VariantParseError("Unterminated string".into()))?;
            if ch == '"' {
                break;
            }
            if ch == '\\' {
                let next = self.get_char().ok_or_else(|| VariantParseError("Unterminated string".into()))?;
                let decoded = match next {
                    'b' => '\u{8}',
                    't' => '\t',
                    'n' => '\n',
                    'f' => '\u{c}',
                    'r' => '\r',
                    'u' => self.parse_hex_escape(4)?,
                    'U' => self.parse_hex_escape(6)?,
                    c => c,
                };
                s.push(decoded);
            } else {
                s.push(ch);
            }
        }
        Ok(s)
    }

    fn parse_hex_escape(&mut self, len: usize) -> Result<char, VariantParseError> {
        let mut v: u32 = 0;
        for _ in 0..len {
            let c = self.get_char().ok_or_else(|| VariantParseError("Unterminated string".into()))?;
            let digit = match c {
                '0'..='9' => c as u32 - '0' as u32,
                'a'..='f' => c as u32 - 'a' as u32 + 10,
                'A'..='F' => c as u32 - 'A' as u32 + 10,
                _ => return Err(VariantParseError("Malformed hex constant in string".into())),
            };
            v = (v << 4) | digit;
        }
        char::from_u32(v).ok_or_else(|| VariantParseError("Invalid Unicode scalar in string".into()))
    }

    fn parse_number(&mut self) -> Result<Token, VariantParseError> {
        let mut token_text = String::new();
        let mut neg = false;
        let first = self.get_char();
        if first == Some('-') {
            neg = true;
            token_text.push('-');
        } else if let Some(c) = first {
            token_text.push(c);
        }
        let mut reading_int = true;
        let mut is_float = false;
        loop {
            let c = self.get_char();
            let c = match c {
                Some(c) => c,
                None => break,
            };
            match (reading_int, c) {
                (true, c) if c.is_ascii_digit() => token_text.push(c),
                (true, '.') => {
                    token_text.push(c);
                    reading_int = false;
                    is_float = true;
                }
                (true, 'e' | 'E') => {
                    token_text.push(c);
                    reading_int = false;
                    is_float = true;
                }
                (false, c) if c.is_ascii_digit() => token_text.push(c),
                (false, 'e' | 'E') => {
                    token_text.push(c);
                    is_float = true;
                }
                (false, '+' | '-') => token_text.push(c),
                _ => {
                    self.put_back(c);
                    break;
                }
            }
        }
        if is_float {
            let f: f64 = token_text.parse().map_err(|_| VariantParseError("Invalid number".into()))?;
            Ok(Token::Number { int: None, float: f })
        } else {
            let i: i64 = token_text.parse().map_err(|_| VariantParseError("Invalid integer".into()))?;
            Ok(Token::Number {
                int: Some(i),
                float: i as f64,
            })
        }
    }
}



// -----------------------------------------------------------------------------
// Parser — recursive descent, mirrors parse_value / _parse_dictionary / _parse_array
// -----------------------------------------------------------------------------


struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Option<Token>,
    realt_is_double: bool,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str, realt_is_double: bool) -> Self {
        Parser {
            lexer: Lexer::new(s),
            current: None,
            realt_is_double,
        }
    }

    fn get_token(&mut self) -> Result<&Token, VariantParseError> {
        if self.current.is_none() {
            self.current = Some(self.lexer.next_token()?);
        }
        Ok(self.current.as_ref().unwrap())
    }

    fn advance(&mut self) -> Result<Token, VariantParseError> {
        let tok = if let Some(t) = self.current.take() {
            t
        } else {
            self.lexer.next_token()?
        };
        self.current = None;
        Ok(tok)
    }

    fn expect(&mut self, want: &str) -> Result<Token, VariantParseError> {
        let t = self.advance()?;
        match &t {
            Token::ParenOpen if want == "(" => {}
            Token::ParenClose if want == ")" => {}
            Token::CurlyOpen if want == "{" => {}
            Token::CurlyClose if want == "}" => {}
            Token::BracketOpen if want == "[" => {}
            Token::BracketClose if want == "]" => {}
            Token::Colon if want == ":" => {}
            Token::Comma if want == "," => {}
            _ => return Err(VariantParseError(format!("Expected '{}'", want))),
        }
        Ok(t)
    }

    fn parse_value(&mut self, token: Token) -> Result<VariantVal, VariantParseError> {
        match token {
            Token::CurlyOpen => self.parse_dictionary(),
            Token::BracketOpen => self.parse_array(),
            Token::Identifier(id) => self.parse_identifier(&id),
            Token::Number { int, float } => {
                if let Some(i) = int {
                    Ok(VariantVal::Int(i))
                } else {
                    Ok(VariantVal::Float(float))
                }
            }
            Token::Str(s) => Ok(VariantVal::String(s)),
            Token::StringName(s) => Ok(VariantVal::StringName(s)),
            Token::Color { r, g, b, a } => Ok(VariantVal::Color(
                r as f32,
                g as f32,
                b as f32,
                a as f32,
            )),
            Token::Eof => Err(VariantParseError("Unexpected EOF".into())),
            _ => Err(VariantParseError(format!("Expected value, got token"))),
        }
    }

    fn parse_dictionary_body(&mut self) -> Result<IndexMap<Box<VariantVal>, Box<VariantVal>>, VariantParseError> {
        let mut map = IndexMap::new();
        loop {
            let tok = self.advance()?;
            if matches!(tok, Token::CurlyClose) {
                break;
            }
            if matches!(tok, Token::Eof) {
                return Err(VariantParseError("Unexpected EOF while parsing dictionary".into()));
            }
            let key = self.parse_value(tok)?;
            self.expect(":")?;
            let val_tok = self.advance()?;
            if matches!(val_tok, Token::Eof) {
                return Err(VariantParseError("Unexpected EOF while parsing dictionary".into()));
            }
            let val = self.parse_value(val_tok)?;
            map.insert(Box::new(key), Box::new(val));
            let next = self.get_token()?;
            if matches!(next, Token::CurlyClose) {
                continue;
            }
            if !matches!(next, Token::Comma) {
                return Err(VariantParseError("Expected '}' or ','".into()));
            }
            self.advance()?; // consume comma
        }
        Ok(map)
    }

    fn parse_dictionary(&mut self) -> Result<VariantVal, VariantParseError> {
        let map = self.parse_dictionary_body()?;
        Ok(VariantVal::Dictionary(None, map))
    }

    fn parse_array_body(&mut self) -> Result<Vec<Box<VariantVal>>, VariantParseError> {
        let mut arr = Vec::new();
        loop {
            if matches!(self.get_token()?, Token::BracketClose) {
                break;
            }
            let tok = self.advance()?;
            if matches!(tok, Token::Eof) {
                return Err(VariantParseError("Unexpected EOF while parsing array".into()));
            }
            let val = self.parse_value(tok)?;
            arr.push(Box::new(val));
            let next = self.get_token()?;
            if matches!(next, Token::BracketClose) {
                break;
            }
            if !matches!(next, Token::Comma) {
                return Err(VariantParseError("Expected ','".into()));
            }
            self.advance()?;
        }
        Ok(arr)
    }

    fn parse_array(&mut self) -> Result<VariantVal, VariantParseError> {
        let arr = self.parse_array_body()?;
        // expect closing bracket
        self.expect("]")?;
        Ok(VariantVal::Array(None, arr))
    }

    fn parse_construct_real(&mut self, count: usize) -> Result<Vec<f64>, VariantParseError> {
        self.expect("(")?;
        let mut args = Vec::new();
        let mut first = true;
        loop {
            if !first {
                let t = self.advance()?;
                if matches!(t, Token::ParenClose) {
                    break;
                }
                if !matches!(t, Token::Comma) {
                    return Err(VariantParseError("Expected ',' or ')' in constructor".into()));
                }
            }
            let t = self.advance()?;
            if first && matches!(t, Token::ParenClose) {
                break;
            }
            let f = match &t {
                Token::Number { int, float } => *float,
                Token::Identifier(id) => {
                    stor_fix(id).ok_or_else(|| VariantParseError("Expected float in constructor".into()))?
                }
                _ => return Err(VariantParseError("Expected float in constructor".into())),
            };
            args.push(f);
            first = false;
        }
        if args.len() != count {
            return Err(VariantParseError(format!(
                "Expected {} arguments for constructor, got {}",
                count,
                args.len()
            )));
        }
        Ok(args)
    }

    fn real_t_from_f64(&self, f: f64) -> RealT {
        if self.realt_is_double {
            RealT::F64(f)
        } else {
            RealT::F32(f as f32)
        }
    }

    fn parse_construct_int(&mut self, count: usize) -> Result<Vec<i32>, VariantParseError> {
        let reals = self.parse_construct_real(count)?;
        reals
            .into_iter()
            .map(|f| i32::try_from(f as i64).map_err(|_| VariantParseError("Integer overflow in constructor".into())))
            .collect()
    }

    fn parse_identifier(&mut self, id: &str) -> Result<VariantVal, VariantParseError> {
        match id {
            "true" => return Ok(VariantVal::Bool(true)),
            "false" => return Ok(VariantVal::Bool(false)),
            "null" | "nil" => return Ok(VariantVal::Nil),
            "inf" => return Ok(VariantVal::Float(f64::INFINITY)),
            "-inf" | "inf_neg" => return Ok(VariantVal::Float(f64::NEG_INFINITY)),
            "nan" => return Ok(VariantVal::Float(f64::NAN)),
            _ => {}
        }

        if id == "Vector2" {
            let a = self.parse_construct_real(2)?;
            return Ok(VariantVal::Vector2(self.real_t_from_f64(a[0]), self.real_t_from_f64(a[1])));
        }
        if id == "Vector2i" {
            let a = self.parse_construct_int(2)?;
            return Ok(VariantVal::Vector2i(a[0], a[1]));
        }
        if id == "Rect2" {
            let a = self.parse_construct_real(4)?;
            return Ok(VariantVal::Rect2(
                (self.real_t_from_f64(a[0]), self.real_t_from_f64(a[1])),
                (self.real_t_from_f64(a[2]), self.real_t_from_f64(a[3])),
            ));
        }
        if id == "Rect2i" {
            let a = self.parse_construct_int(4)?;
            return Ok(VariantVal::Rect2i((a[0], a[1]), (a[2], a[3])));
        }
        if id == "Vector3" {
            let a = self.parse_construct_real(3)?;
            return Ok(VariantVal::Vector3(self.real_t_from_f64(a[0]), self.real_t_from_f64(a[1]), self.real_t_from_f64(a[2])));
        }
        if id == "Vector3i" {
            let a = self.parse_construct_int(3)?;
            return Ok(VariantVal::Vector3i(a[0], a[1], a[2]));
        }
        if id == "Vector4" {
            let a = self.parse_construct_real(4)?;
            return Ok(VariantVal::Vector4(
                self.real_t_from_f64(a[0]),
                self.real_t_from_f64(a[1]),
                self.real_t_from_f64(a[2]),
                self.real_t_from_f64(a[3]),
            ));
        }
        if id == "Vector4i" {
            let a = self.parse_construct_int(4)?;
            return Ok(VariantVal::Vector4i(a[0], a[1], a[2], a[3]));
        }
        if id == "Transform2D" || id == "Matrix32" {
            let a = self.parse_construct_real(6)?;
            return Ok(VariantVal::Transform2d(
                (self.real_t_from_f64(a[0]), self.real_t_from_f64(a[1])),
                (self.real_t_from_f64(a[2]), self.real_t_from_f64(a[3])),
                (self.real_t_from_f64(a[4]), self.real_t_from_f64(a[5])),
            ));
        }
        if id == "Plane" {
            let a = self.parse_construct_real(4)?;
            return Ok(VariantVal::Plane(
                (self.real_t_from_f64(a[0]), self.real_t_from_f64(a[1]), self.real_t_from_f64(a[2])),
                self.real_t_from_f64(a[3]),
            ));
        }
        if id == "Quaternion" || id == "Quat" {
            let a = self.parse_construct_real(4)?;
            return Ok(VariantVal::Quaternion(
                self.real_t_from_f64(a[0]),
                self.real_t_from_f64(a[1]),
                self.real_t_from_f64(a[2]),
                self.real_t_from_f64(a[3]),
            ));
        }
        if id == "AABB" || id == "Rect3" {
            let a = self.parse_construct_real(6)?;
            return Ok(VariantVal::Aabb(
                (self.real_t_from_f64(a[0]), self.real_t_from_f64(a[1]), self.real_t_from_f64(a[2])),
                (self.real_t_from_f64(a[3]), self.real_t_from_f64(a[4]), self.real_t_from_f64(a[5])),
            ));
        }
        if id == "Basis" || id == "Matrix3" {
            let a = self.parse_construct_real(9)?;
            return Ok(VariantVal::Basis(
                (self.real_t_from_f64(a[0]), self.real_t_from_f64(a[1]), self.real_t_from_f64(a[2])),
                (self.real_t_from_f64(a[3]), self.real_t_from_f64(a[4]), self.real_t_from_f64(a[5])),
                (self.real_t_from_f64(a[6]), self.real_t_from_f64(a[7]), self.real_t_from_f64(a[8])),
            ));
        }
        if id == "Transform3D" || id == "Transform" {
            let a = self.parse_construct_real(12)?;
            return Ok(VariantVal::Transform3d(
                (
                    (self.real_t_from_f64(a[0]), self.real_t_from_f64(a[1]), self.real_t_from_f64(a[2])),
                    (self.real_t_from_f64(a[3]), self.real_t_from_f64(a[4]), self.real_t_from_f64(a[5])),
                    (self.real_t_from_f64(a[6]), self.real_t_from_f64(a[7]), self.real_t_from_f64(a[8])),
                ),
                (self.real_t_from_f64(a[9]), self.real_t_from_f64(a[10]), self.real_t_from_f64(a[11])),
            ));
        }
        if id == "Projection" {
            let a = self.parse_construct_real(16)?;
            return Ok(VariantVal::Projection(
                (
                    self.real_t_from_f64(a[0]),
                    self.real_t_from_f64(a[1]),
                    self.real_t_from_f64(a[2]),
                    self.real_t_from_f64(a[3]),
                ),
                (
                    self.real_t_from_f64(a[4]),
                    self.real_t_from_f64(a[5]),
                    self.real_t_from_f64(a[6]),
                    self.real_t_from_f64(a[7]),
                ),
                (
                    self.real_t_from_f64(a[8]),
                    self.real_t_from_f64(a[9]),
                    self.real_t_from_f64(a[10]),
                    self.real_t_from_f64(a[11]),
                ),
                (
                    self.real_t_from_f64(a[12]),
                    self.real_t_from_f64(a[13]),
                    self.real_t_from_f64(a[14]),
                    self.real_t_from_f64(a[15]),
                ),
            ));
        }
        if id == "Color" {
            let a = self.parse_construct_real(4)?;
            return Ok(VariantVal::Color(
                a[0] as f32,
                a[1] as f32,
                a[2] as f32,
                a[3] as f32,
            ));
        }
        if id == "NodePath" {
            self.expect("(")?;
            let t = self.advance()?;
            let s = match &t {
                Token::Str(ss) => ss.clone(),
                _ => return Err(VariantParseError("Expected string as argument for NodePath()".into())),
            };
            self.expect(")")?;
            return Ok(VariantVal::NodePath(s));
        }
        if id == "RID" {
            self.expect("(")?;
            let t = self.advance()?;
            let s = match &t {
                Token::ParenClose => String::new(),
                Token::Number { int, .. } => int.unwrap_or(0).to_string(),
                Token::Identifier(x) => x.clone(),
                _ => return Err(VariantParseError("Expected number as argument or ')'".into())),
            };
            if !matches!(&t, Token::ParenClose) {
                self.expect(")")?;
            }
            return Ok(VariantVal::Rid(s));
        }
        if id == "Signal" {
            self.expect("(")?;
            self.expect(")")?;
            return Ok(VariantVal::Signal);
        }
        if id == "Callable" {
            self.expect("(")?;
            self.expect(")")?;
            return Ok(VariantVal::Callable);
        }
        if id == "Object" {
            return self.parse_object();
        }
        if id == "Resource" || id == "SubResource" || id == "ExtResource" {
            return self.parse_resource(id);
        }
        if id == "Dictionary" {
            return self.parse_typed_dictionary();
        }
        if id == "Array" {
            return self.parse_typed_array();
        }
        if id == "PackedByteArray" || id == "PoolByteArray" || id == "ByteArray" {
            return self.parse_packed_byte_array();
        }
        if id == "PackedInt32Array" || id == "PackedIntArray" || id == "PoolIntArray" || id == "IntArray" {
            let a = self.parse_construct_int_variadic()?;
            return Ok(VariantVal::PackedInt32Array(a));
        }
        if id == "PackedInt64Array" {
            self.expect("(")?;
            let mut args = Vec::new();
            let mut first = true;
            loop {
                if !first {
                    let t = self.advance()?;
                    if matches!(t, Token::ParenClose) {
                        break;
                    }
                    if !matches!(t, Token::Comma) {
                        return Err(VariantParseError("Expected ',' or ')'".into()));
                    }
                }
                let t = self.advance()?;
                if first && matches!(t, Token::ParenClose) {
                    break;
                }
                let i = match &t {
                    Token::Number { int, float } => int.unwrap_or(*float as i64),
                    Token::Identifier(x) => stor_fix(x).map(|f| f as i64).unwrap_or(0),
                    _ => return Err(VariantParseError("Expected number".into())),
                };
                args.push(i);
                first = false;
            }
            return Ok(VariantVal::PackedInt64Array(args));
        }
        if id == "PackedFloat32Array" || id == "PackedRealArray" || id == "PoolRealArray" || id == "FloatArray" {
            let a = self.parse_construct_real_variadic()?;
            return Ok(VariantVal::PackedFloat32Array(a.into_iter().map(|f| f as f32).collect()));
        }
        if id == "PackedFloat64Array" {
            let a = self.parse_construct_real_variadic()?;
            return Ok(VariantVal::PackedFloat64Array(a));
        }
        if id == "PackedStringArray" || id == "PoolStringArray" || id == "StringArray" {
            self.expect("(")?;
            let mut cs = Vec::new();
            let mut first = true;
            loop {
                if !first {
                    let t = self.advance()?;
                    if matches!(t, Token::ParenClose) {
                        break;
                    }
                    if !matches!(t, Token::Comma) {
                        return Err(VariantParseError("Expected ',' or ')'".into()));
                    }
                }
                let t = self.advance()?;
                if first && matches!(t, Token::ParenClose) {
                    break;
                }
                let s = match &t {
                    Token::Str(ss) => ss.clone(),
                    _ => return Err(VariantParseError("Expected string".into())),
                };
                cs.push(s);
                first = false;
            }
            return Ok(VariantVal::PackedStringArray(cs));
        }
        if id == "PackedVector2Array" || id == "PoolVector2Array" || id == "Vector2Array" {
            let a = self.parse_construct_real_variadic()?;
            if a.len() % 2 != 0 {
                return Err(VariantParseError("PackedVector2Array requires even number of components".into()));
            }
            let pairs: Vec<_> = a.chunks(2).map(|c| (self.real_t_from_f64(c[0]), self.real_t_from_f64(c[1]))).collect();
            return Ok(VariantVal::PackedVector2Array(pairs));
        }
        if id == "PackedVector3Array" || id == "PoolVector3Array" || id == "Vector3Array" {
            let a = self.parse_construct_real_variadic()?;
            if a.len() % 3 != 0 {
                return Err(VariantParseError("PackedVector3Array requires multiple of 3 components".into()));
            }
            let triples: Vec<_> = a
                .chunks(3)
                .map(|c| (self.real_t_from_f64(c[0]), self.real_t_from_f64(c[1]), self.real_t_from_f64(c[2])))
                .collect();
            return Ok(VariantVal::PackedVector3Array(triples));
        }
        if id == "PackedVector4Array" || id == "PoolVector4Array" || id == "Vector4Array" {
            let a = self.parse_construct_real_variadic()?;
            if a.len() % 4 != 0 {
                return Err(VariantParseError("PackedVector4Array requires multiple of 4 components".into()));
            }
            let quads: Vec<_> = a
                .chunks(4)
                .map(|c| (self.real_t_from_f64(c[0]), self.real_t_from_f64(c[1]), self.real_t_from_f64(c[2]), self.real_t_from_f64(c[3])))
                .collect();
            return Ok(VariantVal::PackedVector4Array(quads));
        }
        if id == "PackedColorArray" || id == "PoolColorArray" || id == "ColorArray" {
            let a = self.parse_construct_real_variadic()?;
            if a.len() % 4 != 0 {
                return Err(VariantParseError("PackedColorArray requires multiple of 4 components".into()));
            }
            let quads: Vec<_> = a
                .chunks(4)
                .map(|c| (self.real_t_from_f64(c[0]), self.real_t_from_f64(c[1]), self.real_t_from_f64(c[2]), self.real_t_from_f64(c[3])))
                .collect();
            return Ok(VariantVal::PackedColorArray(quads));
        }

        Err(VariantParseError(format!("Unexpected identifier '{}'", id)))
    }

    fn parse_construct_int_variadic(&mut self) -> Result<Vec<i32>, VariantParseError> {
        self.expect("(")?;
        let mut args = Vec::new();
        let mut first = true;
        loop {
            if !first {
                let t = self.advance()?;
                if matches!(t, Token::ParenClose) {
                    break;
                }
                if !matches!(t, Token::Comma) {
                    return Err(VariantParseError("Expected ',' or ')'".into()));
                }
            }
            let t = self.advance()?;
            if first && matches!(t, Token::ParenClose) {
                break;
            }
            let i = match &t {
                Token::Number { int, float } => int.unwrap_or(*float as i64) as i32,
                Token::Identifier(x) => stor_fix(x).map(|f| f as i32).unwrap_or(0),
                _ => return Err(VariantParseError("Expected number".into())),
            };
            args.push(i);
            first = false;
        }
        Ok(args)
    }

    fn parse_construct_real_variadic(&mut self) -> Result<Vec<f64>, VariantParseError> {
        self.expect("(")?;
        let mut args = Vec::new();
        let mut first = true;
        loop {
            if !first {
                let t = self.advance()?;
                if matches!(t, Token::ParenClose) {
                    break;
                }
                if !matches!(t, Token::Comma) {
                    return Err(VariantParseError("Expected ',' or ')'".into()));
                }
            }
            let t = self.advance()?;
            if first && matches!(t, Token::ParenClose) {
                break;
            }
            let f = match &t {
                Token::Number { float, .. } => *float,
                Token::Identifier(x) => stor_fix(x).ok_or_else(|| VariantParseError("Expected number".into()))?,
                _ => return Err(VariantParseError("Expected number".into())),
            };
            args.push(f);
            first = false;
        }
        Ok(args)
    }

    fn parse_object(&mut self) -> Result<VariantVal, VariantParseError> {
        self.expect("(")?;
        let t = self.advance()?;
        let type_name = match &t {
            Token::Identifier(s) => s.clone(),
            _ => return Err(VariantParseError("Expected identifier with type of object".into())),
        };
        self.expect(",")?;
        let mut props = IndexMap::new();
        loop {
            let key_tok = self.advance()?;
            if matches!(key_tok, Token::ParenClose) {
                break;
            }
            if !matches!(key_tok, Token::Str(..)) {
                return Err(VariantParseError("Expected property name as string".into()));
            }
            let key = match key_tok {
                Token::Str(k) => k,
                _ => unreachable!(),
            };
            self.expect(":")?;
            let val_tok = self.advance()?;
            let val = self.parse_value(val_tok)?;
            props.insert(key, Box::new(val));
            let next = self.get_token()?;
            if matches!(next, Token::ParenClose) {
                continue;
            }
            if !matches!(next, Token::Comma) {
                return Err(VariantParseError("Expected '}' or ','".into()));
            }
            self.advance()?;
        }
        Ok(VariantVal::Object(type_name, props))
    }

    fn parse_resource(&mut self, id: &str) -> Result<VariantVal, VariantParseError> {
        self.expect("(")?;
        let t = self.advance()?;
        match id {
            "Resource" => {
                let (path, uid) = match &t {
                    Token::Str(uid_or_path) => {
                        let uid_or_path = uid_or_path.clone();
                        let next = self.get_token()?;
                        if matches!(next, Token::Comma) {
                            self.advance()?;
                            let t2 = self.advance()?;
                            let path = match &t2 {
                                Token::Str(u) => u.clone(),
                                _ => return Err(VariantParseError("Expected string in Resource reference".into())),
                            };
                            (path, Some(uid_or_path))
                        } else {
                            (uid_or_path, None)
                        }
                    }
                    _ => return Err(VariantParseError("Expected string as argument for Resource()".into())),
                };
                self.expect(")")?;
                Ok(VariantVal::Resource(uid, path))
            }
            "SubResource" => {
                let id_str = match &t {
                    Token::Str(s) => s.clone(),
                    _ => return Err(VariantParseError("Expected identifier for SubResource".into())),
                };
                self.expect(")")?;
                Ok(VariantVal::SubResource(id_str))
            }
            "ExtResource" => {
                let id_str = match &t {
                    Token::Str(id) => id.clone(),
                    _ => return Err(VariantParseError("Expected string or identifier for ExtResource".into())),
                };
                self.expect(")")?;
                Ok(VariantVal::ExtResource(id_str))
            }
            _ => unreachable!(),
        }
    }

    fn parse_typed_dictionary(&mut self) -> Result<VariantVal, VariantParseError> {
        self.expect("[")?;
        let _key_type = self.advance()?;
        let key_type = match &_key_type {
            Token::Identifier(s) => self.parse_type_identifier(s)?,
            _ => return Err(VariantParseError("Expected identifier for key type".into())),
        };
        self.expect(",")?;
        let _val_type = self.advance()?;
        let val_type = match &_val_type {
            Token::Identifier(s) => self.parse_type_identifier(s)?,
            _ => return Err(VariantParseError("Expected identifier for value type".into())),
        };
        self.expect("]")?;
        self.expect("(")?;
        self.expect("{")?;
        let map = self.parse_dictionary_body()?;
        self.expect(")")?;
        Ok(VariantVal::Dictionary(Some((Box::new(key_type), Box::new(val_type))), map))
    }
    fn parse_type_identifier(&mut self, s: &str) -> Result<ElemType, VariantParseError> {
        let id = if s == "Resource" || s == "SubResource" || s == "ExtResource" {
            self.parse_identifier(s)
        } else {
            Ok(VariantVal::String(s.to_string()))
        };
        match id {
            Ok(VariantVal::String(s)) => Ok(ElemType::Identifier(s)),
            Ok(VariantVal::Resource(uid, path)) => Ok(ElemType::Resource(uid, path)),
            Ok(VariantVal::SubResource(s)) => Ok(ElemType::SubResource(s)),
            Ok(VariantVal::ExtResource(id)) => Ok(ElemType::ExtResource(id)),
            _ => Err(VariantParseError("Expected identifier for type".into())),
        }
    }

    fn parse_typed_array(&mut self) -> Result<VariantVal, VariantParseError> {
        self.expect("[")?;
        let _elem_type = self.advance()?; // skip type identifier
        let elem_type = match &_elem_type {
            Token::Identifier(s) => self.parse_type_identifier(s)?,
            _ => return Err(VariantParseError("Expected identifier for element type".into())),
        };
        self.expect("]")?;
        self.expect("(")?;
        self.expect("[")?;
        let arr = self.parse_array_body()?;
        self.expect("]")?;
        self.expect(")")?;
        Ok(VariantVal::Array(Some(Box::new(elem_type)), arr))
    }

    fn parse_packed_byte_array(&mut self) -> Result<VariantVal, VariantParseError> {
        self.expect("(")?;
        let t = self.advance()?;
        match &t {
            Token::Str(base64) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(base64.as_bytes())
                    .map_err(|_| VariantParseError("Invalid base64-encoded string".into()))?;
                self.expect(")")?;
                Ok(VariantVal::PackedByteArray(bytes))
            }
            Token::ParenClose => Ok(VariantVal::PackedByteArray(Vec::new())),
            Token::Number { .. } | Token::Identifier(_) => {
                let mut bytes = Vec::new();
                let mut tok = t;
                loop {
                    let b = match &tok {
                        Token::Number { int, float } => int.unwrap_or(*float as i64) as u8,
                        Token::Identifier(x) => stor_fix(x).map(|f| f as u8).unwrap_or(0),
                        _ => return Err(VariantParseError("Expected number in constructor".into())),
                    };
                    bytes.push(b);
                    let next = self.advance()?;
                    if matches!(next, Token::ParenClose) {
                        break;
                    }
                    if !matches!(next, Token::Comma) {
                        return Err(VariantParseError("Expected ',' or ')'".into()));
                    }
                    tok = self.advance()?;
                }
                Ok(VariantVal::PackedByteArray(bytes))
            }
            _ => Err(VariantParseError("Expected base64 string or list of numbers".into())),
        }
    }
}

// -----------------------------------------------------------------------------
// FromStr
// -----------------------------------------------------------------------------

impl VariantVal{
    fn parse_variant(s: &str) -> Result<Self, VariantParseError> {
        let mut parser = Parser::new(s.trim(), REALT_IS_DOUBLE);
        let first = parser.advance()?;
        if matches!(first, Token::Eof) {
            return Err(VariantParseError("Expected value".into()));
        }
        let value = parser.parse_value(first)?;
        let next = parser.get_token()?;
        if !matches!(next, Token::Eof) {
            return Err(VariantParseError("Unexpected trailing input".into()));
        }
        Ok(value)
    }
}

impl FromStr for VariantVal {
    type Err = VariantParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_variant(s)
    }
}

// -----------------------------------------------------------------------------
// Display (variant text format, inverse of FromStr)
// -----------------------------------------------------------------------------

impl Display for ElemType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ElemType::Identifier(s) => write!(f, "{}", s),
            ElemType::Resource(uid, path) => match uid {
                Some(uid) => write!(f, "Resource(\"{}\", \"{}\")", uid, path),
                None => write!(f, "Resource(\"{}\")", path),
            },
            ElemType::SubResource(s) => write!(f, "SubResource(\"{}\")", s),
            ElemType::ExtResource(id) => write!(f, "ExtResource(\"{}\")", id),
        }
    }
}

impl VariantVal {
    pub fn to_string_compat(&self, compat: bool) -> Result<String, std::fmt::Error> {
        let mut output = String::new();
        self.value_to_string_write(&mut output, compat)?;
        Ok(output)
    }

    fn value_to_string_write(&self, f: &mut dyn std::fmt::Write, compat: bool) -> std::fmt::Result {
        use VariantVal::*;

        match self {
            Nil => write!(f, "null"),
            Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            Int(i) => write!(f, "{}", i),
            Float(x) => write!(f, "{}", rtos_fix_impl(*x, compat)),
            String(s) => write!(f, "\"{}\"", escape_string_for_variant(s)),
            Vector2(x, y) => write!(f, "Vector2({}, {})", x.rtos_fix(compat), y.rtos_fix(compat)),
            Vector2i(x, y) => write!(f, "Vector2i({}, {})", x, y),
            Rect2(p, sz) => write!(f, "Rect2({}, {}, {}, {})", p.0.rtos_fix(compat), p.1.rtos_fix(compat), sz.0.rtos_fix(compat), sz.1.rtos_fix(compat)),
            Rect2i((px, py), (sx, sy)) => write!(f, "Rect2i({}, {}, {}, {})", px, py, sx, sy),
            Vector3(x, y, z) => write!(f, "Vector3({}, {}, {})", x.rtos_fix(compat), y.rtos_fix(compat), z.rtos_fix(compat)),
            Vector3i(x, y, z) => write!(f, "Vector3i({}, {}, {})", x, y, z),
            Transform2d(c0, c1, c2) => write!(f, "Transform2D({}, {}, {}, {}, {}, {})", c0.0.rtos_fix(compat), c0.1.rtos_fix(compat), c1.0.rtos_fix(compat), c1.1.rtos_fix(compat), c2.0.rtos_fix(compat), c2.1.rtos_fix(compat)),
            Vector4(x, y, z, w) => write!(f, "Vector4({}, {}, {}, {})", x.rtos_fix(compat), y.rtos_fix(compat), z.rtos_fix(compat), w.rtos_fix(compat)),
            Vector4i(x, y, z, w) => write!(f, "Vector4i({}, {}, {}, {})", x, y, z, w),
            Plane((nx, ny, nz), d) => write!(f, "Plane({}, {}, {}, {})", nx.rtos_fix(compat), ny.rtos_fix(compat), nz.rtos_fix(compat), d.rtos_fix(compat)),
            Quaternion(x, y, z, w) => write!(f, "Quaternion({}, {}, {}, {})", x.rtos_fix(compat), y.rtos_fix(compat), z.rtos_fix(compat), w.rtos_fix(compat)),
            Aabb(p, s) => write!(f, "AABB({}, {}, {}, {}, {}, {})", p.0.rtos_fix(compat), p.1.rtos_fix(compat), p.2.rtos_fix(compat), s.0.rtos_fix(compat), s.1.rtos_fix(compat), s.2.rtos_fix(compat)),
            Basis(r0, r1, r2) => write!(f, "Basis({}, {}, {}, {}, {}, {}, {}, {}, {})", r0.0.rtos_fix(compat), r0.1.rtos_fix(compat), r0.2.rtos_fix(compat), r1.0.rtos_fix(compat), r1.1.rtos_fix(compat), r1.2.rtos_fix(compat), r2.0.rtos_fix(compat), r2.1.rtos_fix(compat), r2.2.rtos_fix(compat)),
            Transform3d((r0, r1, r2), o) => write!(f, "Transform3D({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})", r0.0.rtos_fix(compat), r0.1.rtos_fix(compat), r0.2.rtos_fix(compat), r1.0.rtos_fix(compat), r1.1.rtos_fix(compat), r1.2.rtos_fix(compat), r2.0.rtos_fix(compat), r2.1.rtos_fix(compat), r2.2.rtos_fix(compat), o.0.rtos_fix(compat), o.1.rtos_fix(compat), o.2.rtos_fix(compat)),
            Projection(c0, c1, c2, c3) => write!(f, "Projection({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})", c0.0.rtos_fix(compat), c0.1.rtos_fix(compat), c0.2.rtos_fix(compat), c0.3.rtos_fix(compat), c1.0.rtos_fix(compat), c1.1.rtos_fix(compat), c1.2.rtos_fix(compat), c1.3.rtos_fix(compat), c2.0.rtos_fix(compat), c2.1.rtos_fix(compat), c2.2.rtos_fix(compat), c2.3.rtos_fix(compat), c3.0.rtos_fix(compat), c3.1.rtos_fix(compat), c3.2.rtos_fix(compat), c3.3.rtos_fix(compat)),
            Color(r, g, b, a) => write!(f, "Color({}, {}, {}, {})", rtos_fix_impl(*r, compat), rtos_fix_impl(*g, compat), rtos_fix_impl(*b, compat), rtos_fix_impl(*a, compat)),
            StringName(s) => write!(f, "&\"{}\"", escape_string_for_variant(s)),
            NodePath(s) => write!(f, "NodePath(\"{}\")", escape_string_for_variant(s)),
            Rid(id) => if id.is_empty() { write!(f, "RID()") } else { write!(f, "RID({})", id) },
            Object(ty, props) => {
                write!(f, "Object({}, ", ty)?;
                let mut first = true;
                for (k, v) in props {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "\"{}\": {}", escape_string_for_variant(k), v)?;
                }
                write!(f, ")")
            }
            Callable => write!(f, "Callable()"),
            Signal => write!(f, "Signal()"),
            Dictionary(typed, map) => {
                if let Some((key_type, value_type)) = typed {
                    write!(f, "Dictionary[{}, {}](", key_type.to_string(), value_type.to_string())?;
                }
                write!(f, "{{")?;
                let size = map.len();
                if size > 0 {
                    write!(f, "\n")?;
                }
                for (i, (key, value)) in map.iter().enumerate() {
                    write!(f, "{}: {}", key.to_string_compat(compat)?, value.to_string_compat(compat)?)?;
                    if i < size - 1 {
                        write!(f, ",")?;
                    }
                    write!(f, "\n")?;
                }
                write!(f, "}}")?;
                if typed.is_some() {
                    write!(f, ")")?;
                }
                Ok(())
            }
            Array(elem_type, arr) => {
                if let Some(elem_type) = elem_type {
                    write!(f, "Array[{}](", elem_type.to_string())?;
                }
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")?;
                if elem_type.is_some() {
                    write!(f, ")")?;
                }
                Ok(())
            }
            PackedByteArray(bytes) => {
                write!(f, "PackedByteArray(")?;
                if bytes.is_empty() {
                    write!(f, ")")?;
                } else if compat {
                    // write the bytes as a list of numbers
                    for (i, b) in bytes.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", b)?;
                    }
                } else {
                    write!(f, "\"{}\")", base64::engine::general_purpose::STANDARD.encode(bytes))?;
                }
                Ok(())
            }
            PackedInt32Array(arr) => {
                write!(f, "PackedInt32Array(")?;
                for (i, x) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", x)?;
                }
                write!(f, ")")
            }
            PackedInt64Array(arr) => {
                write!(f, "PackedInt64Array(")?;
                for (i, x) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", x)?;
                }
                write!(f, ")")
            }
            PackedFloat32Array(arr) => {
                write!(f, "PackedFloat32Array(")?;
                for (i, x) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", x.rtos_fix(compat))?;
                }
                write!(f, ")")
            }
            PackedFloat64Array(arr) => {
                write!(f, "PackedFloat64Array(")?;
                for (i, x) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", x.rtos_fix(compat))?;
                }
                write!(f, ")")
            }
            PackedStringArray(arr) => {
                write!(f, "PackedStringArray(")?;
                for (i, s) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\"", escape_string_for_variant(s))?;
                }
                write!(f, ")")
            }
            PackedVector2Array(arr) => {
                write!(f, "PackedVector2Array(")?;
                for (i, (x, y)) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}, {}", x.rtos_fix(compat), y.rtos_fix(compat))?;
                }
                write!(f, ")")
            }
            PackedVector3Array(arr) => {
                write!(f, "PackedVector3Array(")?;
                for (i, (x, y, z)) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}, {}, {}", x.rtos_fix(compat), y.rtos_fix(compat), z.rtos_fix(compat))?;
                }
                write!(f, ")")
            }
            PackedVector4Array(arr) => {
                write!(f, "PackedVector4Array(")?;
                for (i, (x, y, z, w)) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}, {}, {}, {}", x.rtos_fix(compat), y.rtos_fix(compat), z.rtos_fix(compat), w.rtos_fix(compat))?;
                }
                write!(f, ")")
            }
            PackedColorArray(arr) => {
                write!(f, "PackedColorArray(")?;
                for (i, (r, g, b, a)) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}, {}, {}, {}", r.rtos_fix(compat), g.rtos_fix(compat), b.rtos_fix(compat), a.rtos_fix(compat))?;
                }
                write!(f, ")")
            }
            Resource(uid_opt, path) => {
                if let Some(uid) = uid_opt {
                    write!(f, "Resource(\"{}\", \"{}\")", escape_string_for_variant(uid), escape_string_for_variant(path))
                } else {
                    write!(f, "Resource(\"{}\")", escape_string_for_variant(path))
                }
            }
            SubResource(id) => write!(f, "SubResource(\"{}\")", escape_string_for_variant(id)),
            ExtResource(id) => {
                write!(f, "ExtResource(\"{}\")", escape_string_for_variant(id))
            }
        }
        }
}

impl Display for VariantVal {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value_to_string_write(f, true)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    fn test_cases() -> Vec<(&'static str, VariantVal, bool)> {
        let mut map_dict = IndexMap::new();
        map_dict.insert(
            Box::new(VariantVal::String("foo".into())),
            Box::new(VariantVal::String("bar".into())),
        );
        map_dict.insert(
            Box::new(VariantVal::String("baz".into())),
            Box::new(VariantVal::Int(123)),
        );

        let mut map_typed_dict = IndexMap::new();
        map_typed_dict.insert(
            Box::new(VariantVal::String("foo".into())),
            Box::new(VariantVal::Int(123)),
        );
        map_typed_dict.insert(
            Box::new(VariantVal::String("baz".into())),
            Box::new(VariantVal::Int(456)),
        );

        let mut object_props = IndexMap::new();
        object_props.insert("bar".into(), Box::new(VariantVal::Int(123)));

        vec![
            ("null", VariantVal::Nil, true),
            ("nil", VariantVal::Nil, false),
            ("true", VariantVal::Bool(true), true),
            ("false", VariantVal::Bool(false), true),
            ("123", VariantVal::Int(123), true),
            ("123.456", VariantVal::Float(123.456), true),
            ("1.5707964", VariantVal::Float(1.5707964), true),
            // scientific notation
            ("1.23456e+10", VariantVal::Float(1.23456e+10), true),
            ("1.23456e-10", VariantVal::Float(1.23456e-10), true),
            ("inf", VariantVal::Float(f64::INFINITY), true  ),
            ("-inf", VariantVal::Float(f64::NEG_INFINITY), true),
            ("nan", VariantVal::Float(f64::NAN), true),
            ("\"foo\"", VariantVal::String("foo".into()), true),
            ("&\"foo\"", VariantVal::StringName("foo".into()), true),
            ("#ff0000", VariantVal::Color(1.0, 0.0, 0.0, 1.0), false),
            ("#ff000080", VariantVal::Color(1.0, 0.0, 0.0, 128.0 / 255.0), false),
            ("#f00", VariantVal::Color(1.0, 0.0, 0.0, 1.0), false), // 3-digit (Godot Color::html)
            ("#f008", VariantVal::Color(1.0, 0.0, 0.0, 8.0 / 15.0), false), // 4-digit
            ("Vector2(1.0, 2.0)", VariantVal::Vector2(RealT::F64(1.0), RealT::F64(2.0)), true),
            ("Vector2i(1, 2)", VariantVal::Vector2i(1, 2), true),
            ("Rect2(0, 0, 10.0, 10.0)", VariantVal::Rect2(
                (RealT::F64(0.0), RealT::F64(0.0)),
                (RealT::F64(10.0), RealT::F64(10.0)),
            ), true     ),
            ("Rect2i(0, 0, 10, 10)", VariantVal::Rect2i((0, 0), (10, 10)), true ),
            ("Vector3(1.0, 2.0, 3.0)", VariantVal::Vector3(RealT::F64(1.0), RealT::F64(2.0), RealT::F64(3.0)), true),
            ("Vector3i(1, 2, 3)", VariantVal::Vector3i(1, 2, 3), true ),
            ("Vector4(1.0, 2.0, 3.0, 4.0)", VariantVal::Vector4(RealT::F64(1.0), RealT::F64(2.0), RealT::F64(3.0), RealT::F64(4.0)), true ),
            ("Vector4i(1, 2, 3, 4)", VariantVal::Vector4i(1, 2, 3, 4), true ),
            (
                "Transform2D(1.0, 0, 0, 1.0, 0, 0)",
                VariantVal::Transform2d(
                    (RealT::F64(1.0), RealT::F64(0.0)),
                    (RealT::F64(0.0), RealT::F64(1.0)),
                    (RealT::F64(0.0), RealT::F64(0.0)),
                ),
                true,
            ),
            ("Plane(1.0, 0, 0, 0)", VariantVal::Plane(
                (RealT::F64(1.0), RealT::F64(0.0), RealT::F64(0.0)),
                RealT::F64(0.0),
            ), true),
            ("Quaternion(1.0, 0, 0, 0)", VariantVal::Quaternion(
                RealT::F64(1.0),
                RealT::F64(0.0),
                RealT::F64(0.0),
                RealT::F64(0.0),
            ), true),
            (
                "AABB(0, 0, 0, 1.0, 1.0, 1.0)",
                VariantVal::Aabb(
                    (RealT::F64(0.0), RealT::F64(0.0), RealT::F64(0.0)),
                    (RealT::F64(1.0), RealT::F64(1.0), RealT::F64(1.0)),
                ), true
            ),
            (
                "Basis(1.0, 0, 0, 0, 1.0, 0, 0, 0, 1.0)",
                VariantVal::Basis(
                    (RealT::F64(1.0), RealT::F64(0.0), RealT::F64(0.0)),
                    (RealT::F64(0.0), RealT::F64(1.0), RealT::F64(0.0)),
                    (RealT::F64(0.0), RealT::F64(0.0), RealT::F64(1.0)),
                ),
                true,
            ),
            (
                "Transform3D(1.0, 0, 0, 0, 1.0, 0, 0, 0, 1.0, 0, 0, 0)",
                VariantVal::Transform3d(
                    (
                        (RealT::F64(1.0), RealT::F64(0.0), RealT::F64(0.0)),
                        (RealT::F64(0.0), RealT::F64(1.0), RealT::F64(0.0)),
                        (RealT::F64(0.0), RealT::F64(0.0), RealT::F64(1.0)),
                    ),
                    (RealT::F64(0.0), RealT::F64(0.0), RealT::F64(0.0)),
                ),
                true,
            ),
            (
                "Color(1.0, 0, 0, 1.0)",
                VariantVal::Color(1.0, 0.0, 0.0, 1.0),
                true,
            ),
            ("NodePath(\"foo/bar/baz\")", VariantVal::NodePath("foo/bar/baz".into()), true),
            ("RID()", VariantVal::Rid("".into()), true),
            ("RID(42)", VariantVal::Rid("42".into()), true),
            ("Callable()", VariantVal::Callable, true),
            ("Signal()", VariantVal::Signal, true),
            (
                "Object(Node, \"bar\": 123)",
                VariantVal::Object("Node".into(), object_props),
                true,
            ),
            ("{\n\"foo\": \"bar\",\n\"baz\": 123\n}", VariantVal::Dictionary(None, map_dict), true),
            (
                "[1, 2, 3]",
                VariantVal::Array(
                    None,
                    vec![
                        Box::new(VariantVal::Int(1)),
                        Box::new(VariantVal::Int(2)),
                        Box::new(VariantVal::Int(3)),
                    ],
                ),
                true,
            ),
            (
                "Dictionary[String, int]({\n\"foo\": 123,\n\"baz\": 456\n})",
                VariantVal::Dictionary(Some((Box::new(ElemType::Identifier("String".into())), Box::new(ElemType::Identifier("int".into())))), map_typed_dict),
                true,
            ),
            (
                "Array[int]([1, 2, 3])",
                VariantVal::Array(
                    Some(Box::new(ElemType::Identifier("int".into()))),
                    vec![
                        Box::new(VariantVal::Int(1)),
                        Box::new(VariantVal::Int(2)),
                        Box::new(VariantVal::Int(3)),
                    ],
                ),
                true,
            ),
            (
                "PackedByteArray(0, 0, 0, 0, 0)",
                VariantVal::PackedByteArray(vec![0, 0, 0, 0, 0]),
                false,
            ),
            (
                "PackedByteArray(\"AAAAAAA=\")",
                VariantVal::PackedByteArray(vec![0, 0, 0, 0, 0]),
                true,
            ),
            (
                "PackedInt32Array(1, 2, 3)",
                VariantVal::PackedInt32Array(vec![1, 2, 3]),
                true,
            ),
            (
                "PackedInt64Array(1, 2, 3)",
                VariantVal::PackedInt64Array(vec![1, 2, 3]),
                true,
            ),
            (
                "PackedFloat32Array(1.0, 2.0, 3.0)",
                VariantVal::PackedFloat32Array(vec![1.0, 2.0, 3.0]),
                true,
            ),
            (
                "PackedFloat64Array(1.0, 2.0, 3.0)",
                VariantVal::PackedFloat64Array(vec![1.0, 2.0, 3.0]),
                true,
            ),
            (
                "PackedStringArray(\"a\", \"b\", \"c\")",
                VariantVal::PackedStringArray(vec!["a".into(), "b".into(), "c".into()]),
                true,
            ),
            (
                "PackedVector2Array(1.0, 2.0, 3.0, 4.0)",
                VariantVal::PackedVector2Array(vec![
                    (RealT::F64(1.0), RealT::F64(2.0)),
                    (RealT::F64(3.0), RealT::F64(4.0)),
                ]),
                true,
            ),
            (
                "PackedVector3Array(1.0, 2.0, 3.0, 4.0, 5.0, 6.0)",
                VariantVal::PackedVector3Array(vec![
                    (RealT::F64(1.0), RealT::F64(2.0), RealT::F64(3.0)),
                    (RealT::F64(4.0), RealT::F64(5.0), RealT::F64(6.0)),
                ]),
                true,
            ),
            (
                "PackedVector4Array(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0)",
                VariantVal::PackedVector4Array(vec![
                    (RealT::F64(1.0), RealT::F64(2.0), RealT::F64(3.0), RealT::F64(4.0)),
                    (RealT::F64(5.0), RealT::F64(6.0), RealT::F64(7.0), RealT::F64(8.0)),
                ]),
                true,
            ),
            (
                "PackedColorArray(1.0, 0, 0, 1.0, 0, 1.0, 0, 1.0)",
                VariantVal::PackedColorArray(vec![
                    (RealT::F64(1.0), RealT::F64(0.0), RealT::F64(0.0), RealT::F64(1.0)),
                    (RealT::F64(0.0), RealT::F64(1.0), RealT::F64(0.0), RealT::F64(1.0)),
                ]),
                true,
            ),
            (
                "Resource(\"res://bar.tres\")",
                VariantVal::Resource(None, "res://bar.tres".into()),
                true,
            ),
            (
                "Resource(\"uid://5252525252\", \"res://bar.tres\")",
                VariantVal::Resource(Some("uid://5252525252".into()), "res://bar.tres".into()),
                true,
            ),
            ("SubResource(\"foo\")", VariantVal::SubResource("foo".into()), true),
            (
                "ExtResource(\"1_ffe31\")",
                VariantVal::ExtResource("1_ffe31".into()),
                true,
            ),
        ]
    }

    #[test]
    fn test_every_variant_type() {
        for (input, expected, compare_string) in test_cases() {
            let parsed = input.parse::<VariantVal>().unwrap_or_else(|e| {
                panic!("Failed to parse {:?}: {}", input, e);
            });
            assert_eq!(parsed, expected, "input: {:?}", input);
            if compare_string {
                assert_eq!(expected.to_string_compat(false).unwrap(), input, "input: {:?}", input);
            }
        }
    }

    /// Writer and parser Variant::FLOAT (mirrors Godot test_variant.h).
    /// Variant::FLOAT is always 64-bit (f64). Tests max finite double write/parse round-trip.
    #[test]
    fn test_writer_and_parser_float() {
        // Maximum non-infinity double-precision float (same as C++ test).
        let a64: f64 = f64::MAX;
        let a64_str = VariantVal::Float(a64).to_string_compat(true).unwrap();

        assert_eq!(a64_str, "1.7976931348623157e+308", "Writes in scientific notation.");
        assert_ne!(a64_str, "inf", "Should not overflow.");
        assert_ne!(a64_str, "nan", "The result should be defined.");

        // Parse back; loses precision in string form but round-trip value is correct.
        let variant_parsed: VariantVal = a64_str.parse().expect("parse max float");
        let float_parsed = match &variant_parsed {
            VariantVal::Float(f) => *f,
            _ => panic!("expected Float, got {:?}", variant_parsed),
        };
        let expected: f64 = 1.797693134862315708145274237317e+308;
        assert_eq!(float_parsed.to_bits(), expected.to_bits(), "Should parse back.");

        // Approximation of Googol with double-precision float.
        let variant_parsed: VariantVal = "1.0e+100".parse().expect("parse 1.0e+100");
        let float_parsed = match &variant_parsed {
            VariantVal::Float(f) => *f,
            _ => panic!("expected Float, got {:?}", variant_parsed),
        };
        assert_eq!(float_parsed, 1.0e+100, "Should match the double literal.");
    }
}