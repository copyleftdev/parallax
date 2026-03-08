//! PQL lexer — tokenizes raw PQL input into a `Vec<(Token, usize)>`.
//!
//! The second element of each tuple is the byte offset of the token's start
//! in the original input — used for error position reporting (INV-Q05).
//!
//! **Design:** hand-written, no external dependencies.

use crate::error::ParseError;

/// PQL tokens.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // ── Keywords ──────────────────────────────────────────────────────────────
    Find,
    With,
    That,
    Return,
    Limit,
    Shortest,
    Path,
    From,
    To,
    Blast,
    Radius,
    Depth,
    Count,
    And,
    Or,
    Group,
    By,
    In,
    Like,
    Exists,
    Not,
    True,
    False,
    Null,
    // ── Verbs ─────────────────────────────────────────────────────────────────
    Has,
    Is,
    Assigned,
    Allows,
    Uses,
    Contains,
    Manages,
    Connects,
    Protects,
    Exploits,
    Trusts,
    Scans,
    Relates,
    // ── Operators ─────────────────────────────────────────────────────────────
    Eq,    // =
    Ne,    // !=
    Lt,    // <
    Lte,   // <=
    Gt,    // >
    Gte,   // >=
    Bang,  // ! (before verb for negation)
    Star,  // * (wildcard entity filter)
    LParen,
    RParen,
    Comma,
    // ── Literals ──────────────────────────────────────────────────────────────
    StringLit(String),
    Integer(i64),
    Float(f64),
    // ── Identifier ────────────────────────────────────────────────────────────
    Ident(String),
}

/// Tokenize `input`, returning `(token, start_byte_offset)` pairs.
pub fn tokenize(input: &str) -> Result<Vec<(Token, usize)>, ParseError> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let start = i;
        let ch = bytes[i] as char;

        // Skip whitespace.
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // String literal: 'content'
        if ch == '\'' {
            i += 1;
            let str_start = i;
            while i < len && bytes[i] != b'\'' {
                i += 1;
            }
            if i >= len {
                return Err(ParseError::UnterminatedString { pos: start });
            }
            let s = &input[str_start..i];
            i += 1; // consume closing '
            tokens.push((Token::StringLit(s.to_owned()), start));
            continue;
        }

        // Number: integer or float.
        if ch.is_ascii_digit() {
            let num_start = i;
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < len && bytes[i] == b'.' {
                i += 1;
                while i < len && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let f: f64 = input[num_start..i]
                    .parse()
                    .map_err(|_| ParseError::Unexpected {
                        expected: "float".into(),
                        got: input[num_start..i].to_owned(),
                        pos: start,
                    })?;
                tokens.push((Token::Float(f), start));
            } else {
                let n: i64 = input[num_start..i]
                    .parse()
                    .map_err(|_| ParseError::ExpectedInteger { pos: start })?;
                tokens.push((Token::Integer(n), start));
            }
            continue;
        }

        // Identifiers and keywords.
        if ch.is_ascii_alphabetic() || ch == '_' {
            let ident_start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &input[ident_start..i];
            let tok = keyword_or_ident(word);
            tokens.push((tok, start));
            continue;
        }

        // Two-character operators.
        if ch == '!' && i + 1 < len && bytes[i + 1] == b'=' {
            tokens.push((Token::Ne, start));
            i += 2;
            continue;
        }
        if ch == '<' && i + 1 < len && bytes[i + 1] == b'=' {
            tokens.push((Token::Lte, start));
            i += 2;
            continue;
        }
        if ch == '>' && i + 1 < len && bytes[i + 1] == b'=' {
            tokens.push((Token::Gte, start));
            i += 2;
            continue;
        }

        // Single-character operators.
        let tok = match ch {
            '=' => Token::Eq,
            '<' => Token::Lt,
            '>' => Token::Gt,
            '!' => Token::Bang,
            '*' => Token::Star,
            '(' => Token::LParen,
            ')' => Token::RParen,
            ',' => Token::Comma,
            _ => return Err(ParseError::UnexpectedChar { ch, pos: start }),
        };
        tokens.push((tok, start));
        i += 1;
    }

    Ok(tokens)
}

/// Map an identifier string (case-insensitive) to a keyword token or `Ident`.
fn keyword_or_ident(word: &str) -> Token {
    match word.to_ascii_uppercase().as_str() {
        "FIND" => Token::Find,
        "WITH" => Token::With,
        "THAT" => Token::That,
        "RETURN" => Token::Return,
        "LIMIT" => Token::Limit,
        "SHORTEST" => Token::Shortest,
        "PATH" => Token::Path,
        "FROM" => Token::From,
        "TO" => Token::To,
        "BLAST" => Token::Blast,
        "RADIUS" => Token::Radius,
        "DEPTH" => Token::Depth,
        "COUNT" => Token::Count,
        "AND" => Token::And,
        "OR" => Token::Or,
        "GROUP" => Token::Group,
        "BY" => Token::By,
        "IN" => Token::In,
        "LIKE" => Token::Like,
        "EXISTS" => Token::Exists,
        "NOT" => Token::Not,
        "TRUE" => Token::True,
        "FALSE" => Token::False,
        "NULL" => Token::Null,
        "HAS" => Token::Has,
        "IS" => Token::Is,
        "ASSIGNED" => Token::Assigned,
        "ALLOWS" => Token::Allows,
        "USES" => Token::Uses,
        "CONTAINS" => Token::Contains,
        "MANAGES" => Token::Manages,
        "CONNECTS" => Token::Connects,
        "PROTECTS" => Token::Protects,
        "EXPLOITS" => Token::Exploits,
        "TRUSTS" => Token::Trusts,
        "SCANS" => Token::Scans,
        "RELATES" => Token::Relates,
        _ => Token::Ident(word.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_simple_find() {
        let toks: Vec<Token> = tokenize("FIND host").unwrap().into_iter().map(|(t, _)| t).collect();
        assert_eq!(toks, vec![Token::Find, Token::Ident("host".into())]);
    }

    #[test]
    fn tokenize_with_string() {
        let toks: Vec<Token> =
            tokenize("WITH state = 'running'").unwrap().into_iter().map(|(t, _)| t).collect();
        assert_eq!(
            toks,
            vec![Token::With, Token::Ident("state".into()), Token::Eq, Token::StringLit("running".into())]
        );
    }

    #[test]
    fn tokenize_ne_operator() {
        let toks: Vec<Token> =
            tokenize("port != 443").unwrap().into_iter().map(|(t, _)| t).collect();
        assert_eq!(toks, vec![Token::Ident("port".into()), Token::Ne, Token::Integer(443)]);
    }

    #[test]
    fn tokenize_negated_traversal() {
        let toks: Vec<Token> =
            tokenize("THAT !PROTECTS").unwrap().into_iter().map(|(t, _)| t).collect();
        assert_eq!(toks, vec![Token::That, Token::Bang, Token::Protects]);
    }

    #[test]
    fn tokenize_wildcard() {
        let toks: Vec<Token> = tokenize("FIND *").unwrap().into_iter().map(|(t, _)| t).collect();
        assert_eq!(toks, vec![Token::Find, Token::Star]);
    }

    #[test]
    fn unterminated_string_error() {
        assert!(tokenize("WITH name = 'abc").is_err());
    }
}
