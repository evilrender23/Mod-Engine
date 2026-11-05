//! Recursive-descent / precedence-climbing parser for ModScript.

use crate::ast::*;
use crate::lexer::{describe, lex, Tok, Token};
use crate::sym::{intern, Sym};
use std::fmt;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct CompileError {
    pub file: Rc<str>,
    pub line: u32,
    pub msg: String,
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.msg)
    }
}

type PResult<T> = Result<T, CompileError>;

/// Parses a whole `.ms` file: objects, functions and loose init statements.
pub fn parse_file(file: &str, src: &str) -> PResult<FileAst> {
    let mut p = Parser::new(file, src)?;
    let mut ast = FileAst::default();
    while *p.peek() != Tok::Eof {
        if p.at_ident("object") && matches!(p.peek2(), Tok::Ident(_)) {
            ast.objects.push(p.object()?);
            continue;
        }
        if *p.peek() == Tok::Function && matches!(p.peek2(), Tok::Ident(_)) {
            p.bump();
            let name = p.ident("function name")?;
            ast.functions.push(p.func_rest(name)?);
            continue;
        }
        ast.init.push(p.stmt()?);
    }
    Ok(ast)
}

/// Parses a plain list of statements (room creation code, REPL input...).
pub fn parse_stmts(file: &str, src: &str) -> PResult<Vec<Stmt>> {
    let mut p = Parser::new(file, src)?;
    let mut out = Vec::new();
    while *p.peek() != Tok::Eof {
        out.push(p.stmt()?);
    }
    Ok(out)
}

/// Parses a single expression.
/// Parses a variable type on its own, e.g. `num(0, 10)` (used by the editor).
pub fn parse_var_type(src: &str) -> PResult<VarType> {
    let mut p = Parser::new("<type>", src)?;
    let t = p.var_type()?;
    if *p.peek() != Tok::Eof {
        return p.err(format!("unexpected {} after type", describe(p.peek())));
    }
    Ok(t)
}

pub fn parse_expr(file: &str, src: &str) -> PResult<Expr> {
    let mut p = Parser::new(file, src)?;
    let e = p.expr()?;
    if *p.peek() != Tok::Eof {
        return p.err(format!("unexpected {} after expression", describe(p.peek())));
    }
    Ok(e)
}

struct Parser {
    t: Vec<Token>,
    pos: usize,
    file: Rc<str>,
}

#[derive(Copy, Clone)]
enum BinKind {
    Op(BinOp),
    And,
    Or,
    Coalesce,
}

fn bin_info(t: &Tok) -> Option<(u8, BinKind)> {
    use BinKind::*;
    Some(match t {
        Tok::QQ => (1, Coalesce),
        Tok::OrOr => (2, Or),
        Tok::AndAnd => (3, And),
        Tok::Pipe => (4, Op(BinOp::BitOr)),
        Tok::Caret => (5, Op(BinOp::BitXor)),
        Tok::Amp => (6, Op(BinOp::BitAnd)),
        Tok::EqEq => (7, Op(BinOp::Eq)),
        Tok::NotEq => (7, Op(BinOp::Ne)),
        Tok::Lt => (8, Op(BinOp::Lt)),
        Tok::Le => (8, Op(BinOp::Le)),
        Tok::Gt => (8, Op(BinOp::Gt)),
        Tok::Ge => (8, Op(BinOp::Ge)),
        Tok::Shl => (9, Op(BinOp::Shl)),
        Tok::Shr => (9, Op(BinOp::Shr)),
        Tok::Plus => (10, Op(BinOp::Add)),
        Tok::Minus => (10, Op(BinOp::Sub)),
        Tok::Star => (11, Op(BinOp::Mul)),
        Tok::Slash => (11, Op(BinOp::Div)),
        Tok::Percent => (11, Op(BinOp::Mod)),
        Tok::IDiv => (11, Op(BinOp::IDiv)),
        _ => return None,
    })
}

fn is_lvalue(e: &Expr) -> bool {
    matches!(e, Expr::Ident(_) | Expr::Field(..) | Expr::Index(..))
}

impl Parser {
    fn new(file: &str, src: &str) -> PResult<Self> {
        let file: Rc<str> = Rc::from(file);
        let t = lex(src).map_err(|(msg, line)| CompileError { file: file.clone(), line, msg })?;
        Ok(Parser { t, pos: 0, file })
    }

    fn peek(&self) -> &Tok {
        &self.t[self.pos].tok
    }
    fn peek2(&self) -> &Tok {
        &self.t[(self.pos + 1).min(self.t.len() - 1)].tok
    }
    fn line(&self) -> u32 {
        self.t[self.pos].line
    }
    fn bump(&mut self) -> Tok {
        let t = self.t[self.pos].tok.clone();
        if self.pos < self.t.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.bump();
            true
        } else {
            false
        }
    }
    fn err<T>(&self, msg: impl Into<String>) -> PResult<T> {
        Err(CompileError { file: self.file.clone(), line: self.line(), msg: msg.into() })
    }
    fn expect(&mut self, t: Tok, what: &str) -> PResult<()> {
        if self.eat(&t) {
            Ok(())
        } else {
            self.err(format!("expected {what} but found {}", describe(self.peek())))
        }
    }
    fn ident(&mut self, what: &str) -> PResult<Sym> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok(s)
            }
            other => self.err(format!("expected {what} but found {}", describe(&other))),
        }
    }
    fn at_ident(&self, name: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if &*s.name() == name)
    }
    fn semi(&mut self) {
        self.eat(&Tok::Semi);
    }

    // ---------------------------------------------------------------- objects

    fn object(&mut self) -> PResult<ObjectDecl> {
        let line = self.line();
        self.bump(); // `object`
        let name = self.ident("object name")?;
        let parent = if self.eat(&Tok::Colon) { Some(self.ident("parent object name")?) } else { None };
        self.expect(Tok::LBrace, "'{' to start the object body")?;
        let mut props = Vec::new();
        let mut events = Vec::new();
        let mut vars = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Semi => {
                    self.bump();
                }
                Tok::Eof => return self.err(format!("object '{name}' is missing its closing '}}'")),
                Tok::Function => {
                    let l = self.line();
                    self.bump();
                    let n = self.ident("method name")?;
                    let def = self.func_rest(n)?;
                    props.push(Stmt { kind: StmtKind::FuncDecl(n, def), line: l });
                }
                Tok::Ident(s) if &*s.name() == "on" && matches!(self.peek2(), Tok::Ident(_)) => {
                    let l = self.line();
                    self.bump();
                    let ev = self.ident("event name")?;
                    let arg = if self.eat(&Tok::LParen) {
                        if self.eat(&Tok::RParen) {
                            None
                        } else {
                            let e = self.expr()?;
                            self.expect(Tok::RParen, "')'")?;
                            Some(e)
                        }
                    } else {
                        None
                    };
                    let body = self.block()?;
                    let fname = intern(&format!("{name}.{ev}"));
                    events.push(EventDecl {
                        name: ev,
                        arg,
                        line: l,
                        body: Rc::new(FuncDef { name: fname, params: vec![], body, file: self.file.clone(), line: l }),
                    });
                }
                Tok::Ident(n) if *self.peek2() == Tok::Colon => {
                    let l = self.line();
                    self.bump();
                    self.bump(); // ':'
                    let ty = self.var_type()?;
                    if vars.iter().any(|v: &VarDecl| v.name == n) {
                        return self.err(format!("variable '{n}' is declared twice"));
                    }
                    vars.push(VarDecl { name: n, ty, line: l });
                    self.expect(Tok::Eq, "'=' and an initial value (typed variables are written `name: type = value`)")?;
                    let v = self.expr()?;
                    self.semi();
                    props.push(Stmt { kind: StmtKind::Assign(Expr::Ident(n), AssignOp::Set, v), line: l });
                }
                Tok::Ident(_) => {
                    let l = self.line();
                    let target = self.postfix()?;
                    if !is_lvalue(&target) {
                        return self.err("expected a property assignment like `speed = 4`");
                    }
                    self.expect(Tok::Eq, "'=' (object properties are written `name = value`)")?;
                    let v = self.expr()?;
                    self.semi();
                    props.push(Stmt { kind: StmtKind::Assign(target, AssignOp::Set, v), line: l });
                }
                other => {
                    return self.err(format!(
                        "unexpected {} in object body (expected `name = value`, `on <event> {{ }}` or `function`)",
                        describe(&other)
                    ))
                }
            }
        }
        Ok(ObjectDecl { name, parent, vars, props, events, file: self.file.clone(), line })
    }

    /// `num`, `num(0, 10)`, `int(1, null)`, `bool`, `str`, `color`, `sprite`, `object`, `sound`,
    /// `room`, `font`, `list("a", "b")`.
    fn var_type(&mut self) -> PResult<VarType> {
        use crate::value::AssetKind;
        let t = self.ident("a type (num, int, bool, str, color, sprite, object, sound, room, font or list)")?;
        let tname = t.name();
        let args = if self.eat(&Tok::LParen) {
            let mut a = Vec::new();
            while !self.eat(&Tok::RParen) {
                a.push(self.expr()?);
                if !self.eat(&Tok::Comma) {
                    self.expect(Tok::RParen, "')'")?;
                    break;
                }
            }
            Some(a)
        } else {
            None
        };
        let num = |p: &Self, e: &Expr| -> PResult<Option<f64>> {
            match e {
                Expr::Null => Ok(None),
                Expr::Num(n) => Ok(Some(*n)),
                Expr::Unary(UnOp::Neg, b) if matches!(**b, Expr::Num(_)) => match **b {
                    Expr::Num(n) => Ok(Some(-n)),
                    _ => unreachable!(),
                },
                _ => p.err("range limits must be numbers or null"),
            }
        };
        let range = |p: &Self, args: &Option<Vec<Expr>>| -> PResult<(Option<f64>, Option<f64>)> {
            match args.as_deref() {
                None => Ok((None, None)),
                Some([a, b]) => Ok((num(p, a)?, num(p, b)?)),
                Some(_) => p.err(format!("{tname}(min, max) takes two limits")),
            }
        };
        let no_args = |p: &Self, ty: VarType| -> PResult<VarType> {
            if args.is_some() {
                return p.err(format!("type '{tname}' takes no arguments"));
            }
            Ok(ty)
        };
        match &*tname {
            "num" => {
                let (min, max) = range(self, &args)?;
                Ok(VarType::Num { min, max })
            }
            "int" => {
                let (min, max) = range(self, &args)?;
                Ok(VarType::Int { min, max })
            }
            "bool" => no_args(self, VarType::Bool),
            "str" => no_args(self, VarType::Str),
            "color" => no_args(self, VarType::Color),
            "sprite" => no_args(self, VarType::Asset(AssetKind::Sprite)),
            "object" => no_args(self, VarType::Asset(AssetKind::Object)),
            "sound" => no_args(self, VarType::Asset(AssetKind::Sound)),
            "room" => no_args(self, VarType::Asset(AssetKind::Room)),
            "font" => no_args(self, VarType::Asset(AssetKind::Font)),
            "list" => {
                let mut opts = Vec::new();
                for e in args.unwrap_or_default() {
                    match e {
                        Expr::Str(s) => opts.push(s.to_string()),
                        _ => return self.err("list(...) options must be strings"),
                    }
                }
                if opts.is_empty() {
                    return self.err("list(...) needs at least one option");
                }
                Ok(VarType::List(opts))
            }
            other => self.err(format!("unknown type '{other}' (use num, int, bool, str, color, sprite, object, sound, room, font or list)")),
        }
    }

    fn func_rest(&mut self, name: Sym) -> PResult<Rc<FuncDef>> {
        let line = self.line();
        self.expect(Tok::LParen, "'(' after function name")?;
        let mut params = Vec::new();
        if !self.eat(&Tok::RParen) {
            loop {
                let pname = self.ident("parameter name")?;
                let default = if self.eat(&Tok::Eq) { Some(self.expr()?) } else { None };
                params.push(Param { name: pname, default });
                if self.eat(&Tok::RParen) {
                    break;
                }
                self.expect(Tok::Comma, "',' or ')' in parameter list")?;
            }
        }
        let body = self.block()?;
        Ok(Rc::new(FuncDef { name, params, body, file: self.file.clone(), line }))
    }

    fn block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect(Tok::LBrace, "'{'")?;
        let mut out = Vec::new();
        loop {
            match self.peek() {
                Tok::RBrace => {
                    self.bump();
                    return Ok(out);
                }
                Tok::Eof => return self.err("missing '}' before end of file"),
                _ => out.push(self.stmt()?),
            }
        }
    }

    // ---------------------------------------------------------------- statements

    fn stmt(&mut self) -> PResult<Stmt> {
        let line = self.line();
        let kind = match self.peek().clone() {
            Tok::LBrace => StmtKind::Block(self.block()?),
            Tok::Semi => {
                self.bump();
                StmtKind::Block(vec![])
            }
            Tok::Var => {
                let k = self.var_decl()?;
                self.semi();
                k
            }
            Tok::If => {
                self.bump();
                let c = self.expr()?;
                let t = self.stmt()?;
                let e = if self.eat(&Tok::Else) { Some(Box::new(self.stmt()?)) } else { None };
                StmtKind::If(c, Box::new(t), e)
            }
            Tok::While => {
                self.bump();
                let c = self.expr()?;
                let b = self.stmt()?;
                StmtKind::While(c, Box::new(b))
            }
            Tok::Do => {
                self.bump();
                let b = self.stmt()?;
                if self.eat(&Tok::Until) {
                    let c = self.expr()?;
                    self.semi();
                    StmtKind::DoUntil(Box::new(b), c)
                } else if self.eat(&Tok::While) {
                    let c = self.expr()?;
                    self.semi();
                    StmtKind::DoUntil(Box::new(b), Expr::Unary(UnOp::Not, Box::new(c)))
                } else {
                    return self.err("expected 'until' or 'while' after do-block");
                }
            }
            Tok::For => {
                self.bump();
                self.expect(Tok::LParen, "'(' after for")?;
                let init = if *self.peek() == Tok::Semi {
                    None
                } else if *self.peek() == Tok::Var {
                    let l = self.line();
                    Some(Box::new(Stmt { kind: self.var_decl()?, line: l }))
                } else {
                    Some(Box::new(self.simple()?))
                };
                self.expect(Tok::Semi, "';' after for-loop initializer")?;
                let cond = if *self.peek() == Tok::Semi { None } else { Some(self.expr()?) };
                self.expect(Tok::Semi, "';' after for-loop condition")?;
                let step = if *self.peek() == Tok::RParen { None } else { Some(Box::new(self.simple()?)) };
                self.expect(Tok::RParen, "')' to close the for header")?;
                let body = self.stmt()?;
                StmtKind::For(init, cond, step, Box::new(body))
            }
            Tok::Repeat => {
                self.bump();
                let n = self.expr()?;
                let b = self.stmt()?;
                StmtKind::Repeat(n, Box::new(b))
            }
            Tok::Switch => {
                self.bump();
                let v = self.expr()?;
                self.expect(Tok::LBrace, "'{' after switch value")?;
                let mut cases = Vec::new();
                loop {
                    if self.eat(&Tok::RBrace) {
                        break;
                    }
                    let value = if self.eat(&Tok::Case) {
                        let e = self.expr()?;
                        self.expect(Tok::Colon, "':' after case value")?;
                        Some(e)
                    } else if self.eat(&Tok::Default) {
                        self.expect(Tok::Colon, "':' after default")?;
                        None
                    } else {
                        return self.err(format!("expected 'case' or 'default' but found {}", describe(self.peek())));
                    };
                    let mut body = Vec::new();
                    while !matches!(self.peek(), Tok::Case | Tok::Default | Tok::RBrace | Tok::Eof) {
                        body.push(self.stmt()?);
                    }
                    cases.push(SwitchCase { value, body });
                }
                StmtKind::Switch(v, cases)
            }
            Tok::Break => {
                self.bump();
                self.semi();
                StmtKind::Break
            }
            Tok::Continue => {
                self.bump();
                self.semi();
                StmtKind::Continue
            }
            Tok::Exit => {
                self.bump();
                self.semi();
                StmtKind::Exit
            }
            Tok::Return => {
                self.bump();
                // a value must start on the same line: `if dead return` + newline returns nothing
                let ends = matches!(self.peek(), Tok::Semi | Tok::RBrace | Tok::Eof) || self.line() != line;
                let v = if ends { None } else { Some(self.expr()?) };
                self.semi();
                StmtKind::Return(v)
            }
            Tok::With => {
                self.bump();
                let t = self.expr()?;
                let b = self.stmt()?;
                StmtKind::With(t, Box::new(b))
            }
            Tok::Function if matches!(self.peek2(), Tok::Ident(_)) => {
                self.bump();
                let n = self.ident("function name")?;
                let def = self.func_rest(n)?;
                StmtKind::FuncDecl(n, def)
            }
            _ => {
                let s = self.simple()?;
                self.semi();
                return Ok(s);
            }
        };
        Ok(Stmt { kind, line })
    }

    fn var_decl(&mut self) -> PResult<StmtKind> {
        self.bump(); // var
        let mut decls = Vec::new();
        loop {
            let n = self.ident("variable name after 'var'")?;
            let init = if self.eat(&Tok::Eq) { Some(self.expr()?) } else { None };
            decls.push((n, init));
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        Ok(StmtKind::Var(decls))
    }

    /// Assignment, increment or expression statement (no trailing `;`).
    fn simple(&mut self) -> PResult<Stmt> {
        let line = self.line();
        if matches!(self.peek(), Tok::PlusPlus | Tok::MinusMinus) {
            let d = if self.bump() == Tok::PlusPlus { 1.0 } else { -1.0 };
            let t = self.postfix()?;
            if !is_lvalue(&t) {
                return self.err("'++'/'--' needs a variable");
            }
            return Ok(Stmt { kind: StmtKind::Incr(t, d), line });
        }
        let e = self.expr()?;
        let op = match self.peek() {
            Tok::Eq => Some(AssignOp::Set),
            Tok::PlusEq => Some(AssignOp::Op(BinOp::Add)),
            Tok::MinusEq => Some(AssignOp::Op(BinOp::Sub)),
            Tok::StarEq => Some(AssignOp::Op(BinOp::Mul)),
            Tok::SlashEq => Some(AssignOp::Op(BinOp::Div)),
            Tok::PercentEq => Some(AssignOp::Op(BinOp::Mod)),
            Tok::AmpEq => Some(AssignOp::Op(BinOp::BitAnd)),
            Tok::PipeEq => Some(AssignOp::Op(BinOp::BitOr)),
            Tok::CaretEq => Some(AssignOp::Op(BinOp::BitXor)),
            Tok::QQEq => Some(AssignOp::Coalesce),
            _ => None,
        };
        if let Some(op) = op {
            if !is_lvalue(&e) {
                return self.err("left side of an assignment must be a variable, field or array element");
            }
            self.bump();
            let rhs = self.expr()?;
            return Ok(Stmt { kind: StmtKind::Assign(e, op, rhs), line });
        }
        if matches!(self.peek(), Tok::PlusPlus | Tok::MinusMinus) {
            if !is_lvalue(&e) {
                return self.err("'++'/'--' needs a variable");
            }
            let d = if self.bump() == Tok::PlusPlus { 1.0 } else { -1.0 };
            return Ok(Stmt { kind: StmtKind::Incr(e, d), line });
        }
        Ok(Stmt { kind: StmtKind::Expr(e), line })
    }

    // ---------------------------------------------------------------- expressions

    fn expr(&mut self) -> PResult<Expr> {
        let c = self.binary(0)?;
        if self.eat(&Tok::Question) {
            let a = self.expr()?;
            self.expect(Tok::Colon, "':' in ternary expression")?;
            let b = self.expr()?;
            return Ok(Expr::Ternary(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn binary(&mut self, min: u8) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        while let Some((prec, kind)) = bin_info(self.peek()) {
            if prec < min {
                break;
            }
            self.bump();
            let rhs = self.binary(prec + 1)?;
            let (a, b) = (Box::new(lhs), Box::new(rhs));
            lhs = match kind {
                BinKind::Op(op) => Expr::Binary(op, a, b),
                BinKind::And => Expr::And(a, b),
                BinKind::Or => Expr::Or(a, b),
                BinKind::Coalesce => Expr::Coalesce(a, b),
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<Expr> {
        match self.peek() {
            Tok::Minus => {
                self.bump();
                let e = self.unary()?;
                Ok(match e {
                    Expr::Num(n) => Expr::Num(-n),
                    e => Expr::Unary(UnOp::Neg, Box::new(e)),
                })
            }
            Tok::Plus => {
                self.bump();
                self.unary()
            }
            Tok::Bang => {
                self.bump();
                Ok(Expr::Unary(UnOp::Not, Box::new(self.unary()?)))
            }
            Tok::Tilde => {
                self.bump();
                Ok(Expr::Unary(UnOp::BitNot, Box::new(self.unary()?)))
            }
            _ => self.postfix(),
        }
    }

    fn args(&mut self, close: Tok) -> PResult<Vec<Expr>> {
        let mut out = Vec::new();
        loop {
            if self.eat(&close) {
                return Ok(out);
            }
            out.push(self.expr()?);
            if self.eat(&close) {
                return Ok(out);
            }
            self.expect(Tok::Comma, "',' between items")?;
        }
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        loop {
            match self.peek() {
                Tok::LParen => {
                    self.bump();
                    let a = self.args(Tok::RParen)?;
                    e = Expr::Call(Box::new(e), a);
                }
                Tok::LBracket => {
                    self.bump();
                    let i = self.expr()?;
                    self.expect(Tok::RBracket, "']'")?;
                    e = Expr::Index(Box::new(e), Box::new(i));
                }
                Tok::Dot => {
                    self.bump();
                    let name = self.ident("field name after '.'")?;
                    e = Expr::Field(Box::new(e), name);
                }
                _ => return Ok(e),
            }
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let tok = self.peek().clone();
        Ok(match tok {
            Tok::Num(n) => {
                self.bump();
                Expr::Num(n)
            }
            Tok::Str(s) => {
                self.bump();
                Expr::Str(s)
            }
            Tok::True => {
                self.bump();
                Expr::Bool(true)
            }
            Tok::False => {
                self.bump();
                Expr::Bool(false)
            }
            Tok::Null | Tok::NoneKw => {
                self.bump();
                Expr::Null
            }
            Tok::Ident(s) => {
                self.bump();
                Expr::Ident(s)
            }
            Tok::SelfKw => {
                self.bump();
                Expr::SelfE
            }
            Tok::Other => {
                self.bump();
                Expr::Other
            }
            Tok::All => {
                self.bump();
                Expr::All
            }
            Tok::Global => {
                self.bump();
                Expr::Global
            }
            Tok::LParen => {
                self.bump();
                let e = self.expr()?;
                self.expect(Tok::RParen, "')'")?;
                e
            }
            Tok::LBracket => {
                self.bump();
                Expr::Array(self.args(Tok::RBracket)?)
            }
            Tok::LBrace => {
                self.bump();
                let mut fields = Vec::new();
                loop {
                    if self.eat(&Tok::RBrace) {
                        break;
                    }
                    let key = match self.bump() {
                        Tok::Ident(s) => s,
                        Tok::Str(s) => intern(&s),
                        other => return self.err(format!("expected a field name but found {}", describe(&other))),
                    };
                    self.expect(Tok::Colon, "':' after struct field name")?;
                    let v = self.expr()?;
                    fields.push((key, v));
                    if self.eat(&Tok::RBrace) {
                        break;
                    }
                    self.expect(Tok::Comma, "',' between struct fields")?;
                }
                Expr::Struct(fields)
            }
            Tok::Function => {
                self.bump();
                Expr::Func(self.func_rest(intern("<anonymous>"))?)
            }
            other => return self.err(format!("expected an expression but found {}", describe(&other))),
        })
    }
}
