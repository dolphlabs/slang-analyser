mod lexer;
mod token;

pub use lexer::{LexError, Span, Token, lex};
pub use token::TokenKind;
