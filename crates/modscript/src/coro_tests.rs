//! Tests of the resumable coroutine executor (coro.rs).

use crate::*;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Default)]
struct H {
    funcs: HashMap<Sym, Rc<FuncDef>>,
    globals: HashMap<Sym, Value>,
    out: Vec<String>,
}

impl Host for H {
    fn user_function(&self, name: Sym) -> Option<Rc<FuncDef>> {
        self.funcs.get(&name).cloned()
    }
    fn has_native(&self, name: Sym) -> bool {
        &*name.name() == "print"
    }
    fn call_native(&mut self, _name: Sym, _cx: &CallCx, args: Vec<Value>) -> RtResult<Value> {
        self.out.push(args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(" "));
        Ok(Value::Null)
    }
    fn global_get(&mut self, name: Sym) -> Option<Value> {
        self.globals.get(&name).cloned()
    }
    fn global_set(&mut self, name: Sym, v: Value) {
        self.globals.insert(name, v);
    }
    fn inst_get(&mut self, _id: u32, _name: Sym) -> RtResult<Option<Value>> {
        Err(RtError::new("no instances"))
    }
    fn inst_set(&mut self, _id: u32, _name: Sym, _v: Value) -> RtResult<()> {
        Err(RtError::new("no instances"))
    }
    fn inst_alive(&self, _id: u32) -> bool {
        false
    }
    fn instances_of(&mut self, _v: &Value) -> Vec<u32> {
        vec![]
    }
    fn constant(&mut self, _name: Sym) -> Option<Value> {
        None
    }
}

fn host(src: &str) -> H {
    let ast = parse_file("co.ms", src).unwrap_or_else(|e| panic!("{e}"));
    let mut h = H::default();
    for f in ast.functions {
        h.funcs.insert(f.name, f);
    }
    run_stmts(&mut h, &ast.init, &Rc::from("co.ms"), CallCx::global()).unwrap();
    h
}

fn start(h: &mut H, f: &str, args: Vec<Value>) -> Coroutine {
    let def = h.funcs.get(&intern(f)).cloned();
    let v = match def {
        Some(d) => Value::Func(Rc::new(Closure::plain(d, None))),
        None => h.globals.get(&intern(f)).cloned().expect("function"),
    };
    Coroutine::new(h, &v, &CallCx::global(), args).unwrap()
}

/// Resumes until done; returns (suspension calls, return value).
fn drive(h: &mut H, co: &mut Coroutine) -> (Vec<String>, Value) {
    let mut sus = Vec::new();
    for _ in 0..1000 {
        match co.resume(h).unwrap() {
            Resume::Suspended(n, a) => sus.push(format!("{}({})", n, a.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","))),
            Resume::Done(v) => return (sus, v),
        }
    }
    panic!("coroutine never ended");
}

#[test]
fn suspends_and_resumes_in_order() {
    let mut h = host("function p(n) { print(\"a\", n); wait(10); print(\"b\"); yield(); print(\"c\"); return n * 2 }");
    let mut co = start(&mut h, "p", vec![Value::Num(4.0)]);
    assert!(h.out.is_empty(), "nothing runs before the first resume");
    match co.resume(&mut h).unwrap() {
        Resume::Suspended(n, a) => {
            assert_eq!(&*n.name(), "wait");
            assert_eq!(a[0].as_num(), Some(10.0));
        }
        _ => panic!(),
    }
    assert_eq!(h.out, ["a 4"]);
    assert!(matches!(co.resume(&mut h).unwrap(), Resume::Suspended(..)));
    assert_eq!(h.out, ["a 4", "b"]);
    match co.resume(&mut h).unwrap() {
        Resume::Done(v) => assert_eq!(v.as_num(), Some(8.0)),
        _ => panic!(),
    }
    assert!(co.is_done());
    assert_eq!(h.out, ["a 4", "b", "c"]);
}

#[test]
fn loops_keep_their_state_across_suspensions() {
    let src = r#"
function p() {
    var total = 0
    for (var i = 0; i < 3; i++) {
        total += i
        print("for", i)
        wait(1)
    }
    var k = 0
    while (true) {
        k++
        if (k == 2) continue
        if (k > 3) break
        print("while", k)
        yield()
    }
    repeat (2) { print("rep"); wait(5) }
    do { k--; yield() } until (k <= 2)
    for (v in [7, 8]) { print("in", v); yield() }
    for (key, val in { a: 1 }) { print(key, val); yield() }
    if (total == 3) { print("if"); wait(2) } else { print("else") }
    return total
}"#;
    let mut h = host(src);
    let mut co = start(&mut h, "p", vec![]);
    let (sus, v) = drive(&mut h, &mut co);
    assert_eq!(v.as_num(), Some(3.0));
    assert_eq!(h.out, ["for 0", "for 1", "for 2", "while 1", "while 3", "rep", "rep", "in 7", "in 8", "a 1", "if"]);
    assert_eq!(
        sus,
        ["wait(1)", "wait(1)", "wait(1)", "yield()", "yield()", "wait(5)", "wait(5)", "yield()", "yield()", "yield()", "yield()", "yield()", "wait(2)"]
    );
}

#[test]
fn nested_blocks_and_early_exit() {
    let mut h = host("function p() { { print(1); { wait(3); print(2) } } if (true) { exit } print(\"never\") }");
    let mut co = start(&mut h, "p", vec![]);
    let (sus, _) = drive(&mut h, &mut co);
    assert_eq!(sus, ["wait(3)"]);
    assert_eq!(h.out, ["1", "2"]);
}

#[test]
fn closures_and_captured_locals_survive() {
    let src = "var base = 10\nglobal.f = function(step = 1) { var acc = base\n while (acc < base + 3) { acc += step\n print(acc)\n yield() }\n base = acc }";
    let mut h = host(src);
    let mut co = start(&mut h, "f", vec![]);
    let (sus, _) = drive(&mut h, &mut co);
    assert_eq!(sus.len(), 3);
    assert_eq!(h.out, ["11", "12", "13"]);
}

#[test]
fn errors_point_at_the_line_and_stop_the_coroutine() {
    let mut h = host("function p() {\n  yield()\n  var x = no_existe + 1\n}");
    let mut co = start(&mut h, "p", vec![]);
    assert!(matches!(co.resume(&mut h).unwrap(), Resume::Suspended(..)));
    let e = match co.resume(&mut h) {
        Err(e) => e,
        Ok(_) => panic!("expected an error"),
    };
    assert_eq!(e.line, 3, "{e}");
    assert!(e.to_string().contains("in coroutine p"), "{e}");
    assert!(co.is_done());
}

#[test]
fn switch_with_a_wait_is_rejected_with_a_clear_message() {
    let mut h = host("function p() { switch (1) { case 1: wait(1) } }");
    let mut co = start(&mut h, "p", vec![]);
    let e = co.resume(&mut h).err().expect("error");
    assert!(e.msg.contains("switch"), "{e}");
}

#[test]
fn a_user_function_named_wait_is_just_called() {
    let mut h = host("function wait(n) { print(\"mine\", n) }\nfunction p() { wait(2); print(\"after\") }");
    let mut co = start(&mut h, "p", vec![]);
    let (sus, _) = drive(&mut h, &mut co);
    assert!(sus.is_empty());
    assert_eq!(h.out, ["mine 2", "after"]);
}
