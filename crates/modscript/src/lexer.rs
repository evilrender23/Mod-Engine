//! Tokenizer for ModScript source text.

use crate::sym::{intern, Sym};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64),
    Str(Rc<str>),
    Ident(Sym),
    // keywords
    Var,
    If,
    Else,
    While,
    Do,
    Until,
    For,
    Repeat,
    Switch,
    Case,
    Default,
    Break,
    Continue,
    Return,
    Exit,
    With,
    Function,
    True,
    False,
    Null,
    SelfKw,
    Other,
    All,
    NoneKw,
    Global,
    IDiv,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    Dot,
    Question,
    QQ,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    Bang,
    AndAnd,
    OrOr,
    Eq,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    QQEq,
    PlusPlus,
    MinusMinus,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
}

pub fn describe(t: &Tok) -> String {
    match t {
        Tok::Num(n) => format!("number {n}"),
        Tok::Str(s) => format!("string \"{s}\""),
        Tok::Ident(s) => format!("name '{s}'"),
        Tok::Eof => "end of file".into(),
        other => format!("'{}'", tok_text(other)),
    }
}

fn tok_text(t: &Tok) -> &'static str {
    use Tok::*;
    match t {
        Var => "var",
        If => "if",
        Else => "else",
        While => "while",
        Do => "do",
        Until => "until",
        For => "for",
        Repeat => "repeat",
        Switch => "switch",
        Case => "case",
        Default => "default",
        Break => "break",
        Continue => "continue",
        Return => "return",
        Exit => "exit",
        With => "with",
        Function => "function",
        True => "true",
        False => "false",
        Null => "null",
        SelfKw => "self",
        Other => "other",
        All => "all",
        NoneKw => "none",
        Global => "global",
        IDiv => "div",
        LParen => "(",
        RParen => ")",
        LBrace => "{",
        RBrace => "}",
        LBracket => "[",
        RBracket => "]",
        Comma => ",",
        Semi => ";",
        Colon => ":",
        Dot => ".",
        Question => "?",
        QQ => "??",
        Plus => "+",
        Minus => "-",
        Star => "*",
        Slash => "/",
        Percent => "%",
        Amp => "&",
        Pipe => "|",
        Caret => "^",
        Tilde => "~",
        Shl => "<<",
        Shr => ">>",
        Bang => "!",
        AndAnd => "&&",
        OrOr => "||",
        Eq => "=",
        EqEq => "==",
        NotEq => "!=",
        Lt => "<",
        Le => "<=",
        Gt => ">",
        Ge => ">=",
        PlusEq => "+=",
        MinusEq => "-=",
        StarEq => "*=",
        SlashEq => "/=",
        PercentEq => "%=",
        AmpEq => "&=",
        PipeEq => "|=",
        CaretEq => "^=",
        QQEq => "??=",
        PlusPlus => "++",
        MinusMinus => "--",
        _ => "?",
    }
}

fn keyword(s: &str) -> Option<Tok> {
    use Tok::*;
    Some(match s {
        "var" => Var,
        "if" => If,
        "else" => Else,
        "while" => While,
        "do" => Do,
        "until" => Until,
        "for" => For,
        "repeat" => Repeat,
        "switch" => Switch,
        "case" => Case,
        "default" => Default,
        "break" => Break,
        "continue" => Continue,
        "return" => Return,
        "exit" => Exit,
        "with" => With,
        "function" => Function,
        "true" => True,
        "false" => False,
        "null" => Null,
        "self" => SelfKw,
        "other" => Other,
        "all" => All,
        "none" => NoneKw,
        "global" => Global,
        "and" => AndAnd,
        "or" => OrOr,
        "not" => Bang,
        "mod" => Percent,
        "div" => IDiv,
        _ => return None,
    })
}

const OPS: &[(&str, Tok)] = &[
    ("??=", Tok::QQEq),
    ("<<", Tok::Shl),
    (">>", Tok::Shr),
    ("<=", Tok::Le),
    (">=", Tok::Ge),
    ("==", Tok::EqEq),
    ("!=", Tok::NotEq),
    ("&&", Tok::AndAnd),
    ("||", Tok::OrOr),
    ("++", Tok::PlusPlus),
    ("--", Tok::MinusMinus),
    ("+=", Tok::PlusEq),
    ("-=", Tok::MinusEq),
    ("*=", Tok::StarEq),
    ("/=", Tok::SlashEq),
    ("%=", Tok::PercentEq),
    ("&=", Tok::AmpEq),
    ("|=", Tok::PipeEq),
    ("^=", Tok::CaretEq),
    ("??", Tok::QQ),
    ("(", Tok::LParen),
    (")", Tok::RParen),
    ("{", Tok::LBrace),
    ("}", Tok::RBrace),
    ("[", Tok::LBracket),
    ("]", Tok::RBracket),
    (",", Tok::Comma),
    (";", Tok::Semi),
    (":", Tok::Colon),
    (".", Tok::Dot),
    ("?", Tok::Question),
    ("+", Tok::Plus),
    ("-", Tok::Minus),
    ("*", Tok::Star),
    ("/", Tok::Slash),
    ("%", Tok::Percent),
    ("&", Tok::Amp),
    ("|", Tok::Pipe),
    ("^", Tok::Caret),
    ("~", Tok::Tilde),
    ("!", Tok::Bang),
    ("=", Tok::Eq),
    ("<", Tok::Lt),
    (">", Tok::Gt),
];

pub fn lex(src: &str) -> Result<Vec<Token>, (String, u32)> {
    let c: Vec<char> = src.chars().collect();
    let n = c.len();
    let at = |i: usize| if i < n { c[i] } else { '\0' };
    let mut i = 0;
    let mut line = 1u32;
    let mut out = Vec::new();

    while i < n {
        let ch = c[i];
        if ch == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '/' && at(i + 1) == '/' {
            while i < n && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '/' && at(i + 1) == '*' {
            let start = line;
            i += 2;
            loop {
                if i >= n {
                    return Err(("unterminated /* comment".into(), start));
                }
                if c[i] == '\n' {
                    line += 1;
                }
                if c[i] == '*' && at(i + 1) == '/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // numbers
        if ch.is_ascii_digit() || (ch == '.' && at(i + 1).is_ascii_digit()) {
            if ch == '0' && (at(i + 1) == 'x' || at(i + 1) == 'X') {
                let start = i + 2;
                i = start;
                while i < n && (c[i].is_ascii_hexdigit() || c[i] == '_') {
                    i += 1;
                }
                let s: String = c[start..i].iter().filter(|&&d| d != '_').collect();
                let v = u64::from_str_radix(&s, 16).map_err(|_| ("bad hex number".to_string(), line))?;
                out.push(Token { tok: Tok::Num(v as f64), line });
                continue;
            }
            let start = i;
            while i < n && (c[i].is_ascii_digit() || c[i] == '_') {
                i += 1;
            }
            if at(i) == '.' && at(i + 1).is_ascii_digit() {
                i += 1;
                while i < n && (c[i].is_ascii_digit() || c[i] == '_') {
                    i += 1;
                }
            }
            if (at(i) == 'e' || at(i) == 'E')
                && (at(i + 1).is_ascii_digit() || ((at(i + 1) == '-' || at(i + 1) == '+') && at(i + 2).is_ascii_digit()))
            {
                i += 2;
                while i < n && c[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let s: String = c[start..i].iter().filter(|&&d| d != '_').collect();
            let v: f64 = s.parse().map_err(|_| (format!("bad number '{s}'"), line))?;
            out.push(Token { tok: Tok::Num(v), line });
            continue;
        }

        // #RRGGBB colour literal
        if ch == '#' {
            let start = i + 1;
            i = start;
            while i < n && c[i].is_ascii_hexdigit() {
                i += 1;
            }
            if i - start != 6 {
                return Err(("colour literals must look like #RRGGBB".into(), line));
            }
            let s: String = c[start..i].iter().collect();
            let v = u32::from_str_radix(&s, 16).unwrap();
            out.push(Token { tok: Tok::Num(v as f64), line });
            continue;
        }

        // strings
        if ch == '"' || ch == '\'' {
            let quote = ch;
            let start_line = line;
            i += 1;
            let mut s = String::new();
            loop {
                if i >= n {
                    return Err(("unterminated string".into(), start_line));
                }
                let d = c[i];
                if d == quote {
                    i += 1;
                    break;
                }
                if d == '\n' {
                    line += 1;
                }
                if d == '\\' {
                    i += 1;
                    let e = at(i);
                    match e {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '0' => s.push('\0'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        '\'' => s.push('\''),
                        '\n' => {
                            line += 1;
                        }
                        other => {
                            return Err((format!("unknown escape '\\{other}'"), line));
                        }
                    }
                    i += 1;
                    continue;
                }
                s.push(d);
                i += 1;
            }
            out.push(Token { tok: Tok::Str(Rc::from(s.as_str())), line: start_line });
            continue;
        }

        // identifiers / keywords
        if ch.is_alphabetic() || ch == '_' {
            let start = i;
            while i < n && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            let s: String = c[start..i].iter().collect();
            let tok = keyword(&s).unwrap_or_else(|| Tok::Ident(intern(&s)));
            out.push(Token { tok, line });
            continue;
        }

        // operators
        let mut matched = false;
        for (text, tok) in OPS {
            let len = text.len();
            if i + len <= n && text.chars().enumerate().all(|(k, tc)| c[i + k] == tc) {
                out.push(Token { tok: tok.clone(), line });
                i += len;
                matched = true;
                break;
            }
        }
        if !matched {
            return Err((format!("unexpected character '{ch}'"), line));
        }
    }
    out.push(Token { tok: Tok::Eof, line });
    Ok(out)
}
