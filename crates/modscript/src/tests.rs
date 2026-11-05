use crate::*;
use std::collections::HashMap;
use std::rc::Rc;

/// Minimal host: a few fake instances, an `out` buffer and a couple of builtins.
#[derive(Default)]
struct TestHost {
    funcs: HashMap<Sym, Rc<FuncDef>>,
    globals: HashMap<Sym, Value>,
    insts: HashMap<u32, HashMap<Sym, Value>>,
    out: Vec<String>,
}

impl Host for TestHost {
    fn user_function(&self, name: Sym) -> Option<Rc<FuncDef>> {
        self.funcs.get(&name).cloned()
    }
    fn has_native(&self, name: Sym) -> bool {
        matches!(&*name.name(), "print" | "len")
    }
    fn call_native(&mut self, name: Sym, _cx: &CallCx, args: Vec<Value>) -> RtResult<Value> {
        match &*name.name() {
            "print" => {
                self.out.push(args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(" "));
                Ok(Value::Null)
            }
            "len" => match &args[0] {
                Value::Array(a) => Ok(Value::Num(a.borrow().len() as f64)),
                Value::Str(s) => Ok(Value::Num(s.chars().count() as f64)),
                _ => Err(RtError::new("len of what?")),
            },
            _ => unreachable!(),
        }
    }
    fn global_get(&mut self, name: Sym) -> Option<Value> {
        self.globals.get(&name).cloned()
    }
    fn global_set(&mut self, name: Sym, v: Value) {
        self.globals.insert(name, v);
    }
    fn inst_get(&mut self, id: u32, name: Sym) -> RtResult<Option<Value>> {
        match self.insts.get(&id) {
            Some(vars) => Ok(vars.get(&name).cloned()),
            None => Err(RtError::new("no such instance")),
        }
    }
    fn inst_set(&mut self, id: u32, name: Sym, v: Value) -> RtResult<()> {
        self.insts.get_mut(&id).ok_or_else(|| RtError::new("no such instance"))?.insert(name, v);
        Ok(())
    }
    fn inst_alive(&self, id: u32) -> bool {
        self.insts.contains_key(&id)
    }
    fn instances_of(&mut self, v: &Value) -> Vec<u32> {
        match v {
            Value::Inst(id) => vec![*id],
            Value::All => {
                let mut ids: Vec<u32> = self.insts.keys().copied().collect();
                ids.sort();
                ids
            }
            _ => vec![],
        }
    }
    fn constant(&mut self, name: Sym) -> Option<Value> {
        match &*name.name() {
            "pi" => Some(Value::Num(std::f64::consts::PI)),
            _ => None,
        }
    }
}

fn run(src: &str) -> Vec<String> {
    run_host(src).out
}

fn run_host(src: &str) -> TestHost {
    let ast = parse_file("test.ms", src).unwrap_or_else(|e| panic!("{e}"));
    let mut h = TestHost::default();
    h.insts.insert(1, HashMap::new());
    h.insts.insert(2, HashMap::new());
    for f in ast.functions {
        h.funcs.insert(f.name, f);
    }
    if let Err(e) = run_stmts(&mut h, &ast.init, &Rc::from("test.ms"), CallCx::global()) {
        panic!("runtime error: {e}");
    }
    h
}

fn run_err(src: &str) -> String {
    let ast = parse_file("test.ms", src).unwrap();
    let mut h = TestHost::default();
    for f in ast.functions {
        h.funcs.insert(f.name, f);
    }
    run_stmts(&mut h, &ast.init, &Rc::from("test.ms"), CallCx::global()).unwrap_err().to_string()
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(run("print(1 + 2 * 3, (1 + 2) * 3, 7 div 2, 7 mod 3, -2 * -3, 2 + 3 == 5)"), ["7 9 3 1 6 true"]);
    assert_eq!(run("print(0.1 + 0.2, 10 / 4, 1 << 4, 0xff, #ff0000)"), ["0.3 2.5 16 255 16711680"]);
    assert_eq!(run("print(\"a\" + 1 + 2, 1 + 2 + \"a\", \"ab\" * 3)"), ["a12 3a ababab"]);
}

#[test]
fn logic_and_ternary() {
    assert_eq!(run("print(true and false, true or false, not true, 1 > 2 ? \"y\" : \"n\", null ?? 5)"), ["false true false n 5"]);
}

#[test]
fn variables_and_loops() {
    let src = r#"
        var total = 0
        for (var i = 0; i < 10; i++) { if i == 5 continue; total += i }
        print(total)
        var n = 0
        while n < 100 { n += 7; if n > 30 break }
        print(n)
        var r = 0
        repeat 4 { r += 2 }
        print(r)
        var d = 0
        do { d++ } until d >= 3
        print(d)
    "#;
    assert_eq!(run(src), ["40", "35", "8", "3"]);
}

#[test]
fn functions_defaults_recursion() {
    let src = r#"
        function fib(n) { if n < 2 return n; return fib(n - 1) + fib(n - 2) }
        function greet(name, greeting = "hola") { return greeting + " " + name }
        function early(flag) {
            if flag return
            return "late"
        }
        print(fib(15), greet("mundo"), greet("x", "adios"), early(true), early(false))
        var sq = function(v) { return v * v }
        print(sq(9))
    "#;
    assert_eq!(run(src), ["610 hola mundo adios x null late", "81"]);
}

#[test]
fn arrays_and_structs() {
    let src = r#"
        var a = [1, 2, 3]
        a[5] = 9
        print(a, len(a))
        var p = { name: "slime", hp: 3, hit: function(d) { hp -= d; return hp } }
        p.hit(2)
        print(p.hp, p["name"])
        grid[2] = "auto"
        print(grid)
        var nested = { list: [1, { deep: true }] }
        nested.list[1].deep = false
        print(nested)
    "#;
    assert_eq!(
        run(src),
        ["[1, 2, 3, null, null, 9] 6", "1 slime", "[null, null, \"auto\"]", "{list: [1, {deep: false}]}"]
    );
}

#[test]
fn switch_fallthrough() {
    let src = r#"
        function f(v) {
            var s = ""
            switch v { case 1: s += "one "; case 2: s += "two "; break; case "x": s = "ex"; break; default: s = "other" }
            return s
        }
        print(f(1) + "|" + f(2) + "|" + f("x") + "|" + f(9))
    "#;
    assert_eq!(run(src), ["one two |two |ex|other"]);
}

#[test]
fn with_and_instances() {
    let src = r#"
        with all { hp = 10 }
        with all { hp -= other_bonus ?? 1 }
        var sum = 0
        with all { sum += hp }
        print(sum)
    "#;
    // `other_bonus` is unknown → reading it errors, so use a struct instead
    let src_ok = src.replace("other_bonus ?? 1", "1");
    assert_eq!(run(&src_ok), ["18"]);
}

#[test]
fn global_and_self_scope() {
    let h = run_host("score = 5\nglobal.lives = 3\nscore += global.lives");
    assert_eq!(h.globals[&intern("score")].to_string(), "8");
}

#[test]
fn error_has_location() {
    let e = run_err("var a = 1\nvar b = a + undefined_thing");
    assert!(e.starts_with("test.ms:2:"), "{e}");
    assert!(e.contains("undefined_thing"), "{e}");
    let e = run_err("function boom() { return 1 / 0 }\n\nboom()");
    assert!(e.contains("division by zero"), "{e}");
}

#[test]
fn parse_object_declaration() {
    let ast = parse_file(
        "o.ms",
        r#"
        object Player : Actor {
            sprite = spr_player
            hp = 3
            function hurt(n) { hp -= n }
            on create { hp = 5 }
            on collision(Coin) { score += 1 }
            on key_pressed(key.space) { jump() }
        }
        "#,
    )
    .unwrap();
    let o = &ast.objects[0];
    assert_eq!(&*o.name.name(), "Player");
    assert_eq!(o.parent.map(|p| p.name().to_string()).as_deref(), Some("Actor"));
    assert_eq!(o.props.len(), 3);
    assert_eq!(o.events.len(), 3);
    assert_eq!(&*o.events[1].name.name(), "collision");
}

#[test]
fn compile_errors_are_readable() {
    let e = parse_file("bad.ms", "object A {\n  on step { x = }\n}").unwrap_err();
    assert_eq!(e.line, 2);
    let e = parse_file("bad.ms", "var s = \"unterminated").unwrap_err();
    assert!(e.msg.contains("unterminated"));
}

#[test]
fn typed_object_variables() {
    let ast = parse_file(
        "t.ms",
        r#"
        object Enemy {
            speed: num(0, 10) = 2
            hp: int(1, null) = 3
            boss: bool = false
            tint: color = #ff8800
            drops: object = Coin
            mode: list("patrol", "chase") = "patrol"
            plain = 5
        }
        "#,
    )
    .unwrap();
    let o = &ast.objects[0];
    assert_eq!(o.vars.len(), 6);
    assert_eq!(o.props.len(), 7, "typed vars are also initialisers");
    assert_eq!(o.vars[0].ty, VarType::Num { min: Some(0.0), max: Some(10.0) });
    assert_eq!(o.vars[1].ty.to_source(), "int(1, null)");
    assert_eq!(o.vars[5].ty, VarType::List(vec!["patrol".into(), "chase".into()]));
    assert_eq!(parse_var_type("num(-1, 1)").unwrap().to_source(), "num(-1, 1)");

    let t = VarType::Num { min: Some(0.0), max: Some(10.0) };
    assert!(t.check(&Value::Num(4.0)).is_ok());
    assert!(t.check(&Value::Num(11.0)).unwrap_err().contains("outside"));
    assert!(t.check(&Value::str("x")).is_err());
    assert!(VarType::Int { min: None, max: None }.check(&Value::Num(1.5)).is_err());
    assert!(VarType::List(vec!["a".into()]).check(&Value::str("b")).is_err());

    assert!(parse_file("t.ms", "object A { s: wat = 1 }").unwrap_err().msg.contains("unknown type"));
    assert!(parse_file("t.ms", "object A { s: num }").unwrap_err().msg.contains("initial value"));
    assert!(parse_file("t.ms", "object A { s: num = 1\n s: num = 2 }").unwrap_err().msg.contains("twice"));
}
