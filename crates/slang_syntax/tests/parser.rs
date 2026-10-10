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

#[test]
fn should_parse_gc_struct_extern_and_link() {
    // Regression: the `gc`/`extern` arms once ate `struct`/`fn` and then
    // the sub-parser ate the declared name instead.
    let f = parse_src(
        "pub gc struct Value {\n    kind: int,\n    document: opt[Document],\n}\nlink \"sqlite3\";\nextern fn sqlite3_libversion() -> str;\n",
    );
    assert_eq!(f.items.len(), 3);
    assert!(
        matches!(&f.items[0], slang_syntax::Item::Struct(s) if s.name == "Value" && s.gc && s.pub_)
    );
    assert!(matches!(&f.items[1], slang_syntax::Item::Link { .. }));
    assert!(
        matches!(&f.items[2], slang_syntax::Item::Fn(f) if f.name == "sqlite3_libversion" && f.extern_)
    );
}

#[test]
fn should_parse_map_value_after_bracket() {
    let f = parse_src("struct C {\n    params: map[str]str,\n    counts: map[str]int,\n}\n");
    assert_eq!(f.items.len(), 1);
}

#[test]
fn should_parse_open_slices() {
    let f = parse_src("fn f(b: bytes) -> bytes {\n    return b[4..] + b[..2] + b[1..3];\n}\n");
    assert_eq!(f.items.len(), 1);
}

#[test]
fn should_parse_assignment_first_if_body() {
    // Regression: `if b { s = 1; }` was read as struct literal `b { s = 1 }`.
    let f = parse_src("fn f(b: bool) {\n    let s = \"false\";\n    if b {\n        s = \"true\";\n    }\n    while b {\n        s = \"x\";\n    }\n}\n");
    assert_eq!(f.items.len(), 1);
}

#[test]
fn should_still_parse_colon_struct_literals() {
    let f = parse_src(
        "fn f() {\n    let p = Point { x: 1, y: 2 };\n    let q = Box[int] { v: 3 };\n}\n",
    );
    assert_eq!(f.items.len(), 1);
}

#[test]
fn should_parse_borrow_link_wire_until_types() {
    let f = parse_src("fn read_frame(c: &mut link, buf: wire, d: until) -> result[int, str] {\n    return ok(1);\n}\n");
    assert_eq!(f.items.len(), 1);
}
