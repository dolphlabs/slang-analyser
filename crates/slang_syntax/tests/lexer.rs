use slang_syntax::{lex, TokenKind};

#[test]
fn should_lex_hello_program() {
    let toks = lex("let n = 10;\nprintln(\"hi ${n}\");\n").unwrap();
    assert_eq!(toks[0].kind, TokenKind::KwLet);
    assert_eq!(toks[1].kind, TokenKind::Ident);
    assert_eq!(toks[2].kind, TokenKind::Assign);
    assert_eq!(toks[3].kind, TokenKind::Int);
    assert_eq!(toks[3].int_val, 10);
}

#[test]
fn should_flag_big_u64_literal() {
    let toks = lex("let mask = 18446744073709551615;").unwrap();
    let int = toks.iter().find(|t| t.kind == TokenKind::Int).unwrap();
    assert!(int.big_u64);
}

#[test]
fn should_reject_overflow_literal() {
    assert!(lex("let x = 99999999999999999999999;").is_err());
}

#[test]
fn should_lex_keywords_and_types() {
    let toks = lex("fn f(a: int) -> int { return a; }").unwrap();
    assert_eq!(toks[0].kind, TokenKind::KwFn);
    assert!(toks.iter().any(|t| t.kind == TokenKind::TyInt));
    assert!(toks.iter().any(|t| t.kind == TokenKind::KwReturn));
}

#[test]
fn should_lex_ranges_and_ops() {
    let toks = lex("for k in 1..=3 { x <<= 1; y ?? 0; }").unwrap();
    assert!(toks.iter().any(|t| t.kind == TokenKind::DotDotEq));
    assert!(toks.iter().any(|t| t.kind == TokenKind::ShlEq));
    assert!(toks.iter().any(|t| t.kind == TokenKind::QuestionQuestion));
}

#[test]
fn should_lex_bytes_and_strings() {
    let toks = lex("let b = b\"hi\\x00\"; let s = \"a${x}b\";").unwrap();
    let b = toks.iter().find(|t| t.kind == TokenKind::Bytes).unwrap();
    assert_eq!(b.bytes.as_ref().unwrap(), &vec![b'h', b'i', 0]);
    let s = toks.iter().find(|t| t.kind == TokenKind::String).unwrap();
    assert!(s.text.as_ref().unwrap().contains('\x01'));
}

#[test]
fn should_skip_line_comments() {
    let toks = lex("// hello\nlet x = 1;").unwrap();
    assert_eq!(toks[0].kind, TokenKind::KwLet);
    assert_eq!(toks[0].span.line, 2);
}
