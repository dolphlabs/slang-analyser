use slang_syntax::{lex, parse};

fn parse_src(src: &str) -> slang_syntax::File {
    let toks = lex(src).unwrap();
    let (file, errors) = parse(toks);
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    file
}

#[test]
fn should_parse_hello_items() {
    let f = parse_src("fn fib(n: int) -> int { return n; }\nfor k in 1..=3 { println(\"x\"); }\n");
    assert_eq!(f.items.len(), 1);
    assert_eq!(f.body.len(), 1);
}

#[test]
fn should_parse_struct_impl_enum() {
    let f = parse_src(
        "struct Point { x: int, y: int }\nimpl Point { fn sum(self: Point) -> int { return self.x; } }\nenum Status { Pending, Paid }\n",
    );
    assert_eq!(f.items.len(), 3);
}

#[test]
fn should_parse_guard_and_if_let() {
    let f = parse_src(
        "fn f() -> int { guard let x = g() else { return -1; } if let y = h() { println(y); } return x; }\n",
    );
    assert_eq!(f.items.len(), 1);
}

#[test]
fn should_parse_switch_and_select() {
    let f = parse_src(
        "fn f(code: int) { switch code { case 200 { println(\"ok\"); } default { println(\"x\"); } } }\n",
    );
    assert_eq!(f.items.len(), 1);
}

#[test]
fn should_parse_generics_and_fn_values() {
    let f = parse_src(
        "struct Box[T] { v: T }\nfn first[T](xs: [T]) -> T { return xs[0]; }\nlet f: fn(int) -> int = double;\n",
    );
    assert_eq!(f.items.len(), 3);
}

#[test]
fn should_recover_from_broken_code() {
    let toks = lex("fn broken( { let x = ; } fn ok() { return 1; }").unwrap();
    let (file, errors) = parse(toks);
    assert!(!errors.is_empty());
    // Recovery still finds the second function.
    assert!(file
        .items
        .iter()
        .any(|i| matches!(i, slang_syntax::Item::Fn(f) if f.name == "ok")));
}
