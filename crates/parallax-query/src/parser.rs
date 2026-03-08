//! PQL recursive-descent parser.
//!
//! Converts a token stream (from the lexer) into a typed AST.
//! All structural invariants are checked here — if it parses, it's valid.
//!
//! **Spec reference:** `specs/04-query-language.md` §4.3, §4.4

use parallax_core::property::Value;

use crate::ast::{
    BlastQuery, EntityFilter, FindQuery, GroupByClause, PathQuery, PropertyCondition, Query,
    ReturnClause, TraversalStep, Verb,
};
use crate::error::ParseError;
use crate::lexer::Token;

/// Parse a PQL string into a `Query` AST.
pub fn parse(input: &str) -> Result<Query, ParseError> {
    let tokens = crate::lexer::tokenize(input)?;
    let mut p = Parser::new(&tokens);
    let query = p.parse_query()?;
    // Reject trailing tokens — they indicate a malformed query.
    if let Some(tok) = p.peek() {
        return Err(ParseError::Unexpected {
            expected: "end of input".to_owned(),
            got: format!("{tok:?}"),
            pos: p.current_pos(),
        });
    }
    Ok(query)
}

struct Parser<'t> {
    tokens: &'t [(Token, usize)],
    pos: usize,
}

impl<'t> Parser<'t> {
    fn new(tokens: &'t [(Token, usize)]) -> Self {
        Parser { tokens, pos: 0 }
    }

    // ─── Token navigation ────────────────────────────────────────────────────

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|(t, _)| t)
    }

    fn pos_of(&self, offset: usize) -> usize {
        self.tokens.get(offset).map(|(_, p)| *p).unwrap_or(usize::MAX)
    }

    fn current_pos(&self) -> usize {
        self.pos_of(self.pos)
    }

    fn advance(&mut self) -> Option<&Token> {
        let tok = self.tokens.get(self.pos).map(|(t, _)| t);
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: Token, expected_desc: &str) -> Result<(), ParseError> {
        match self.peek() {
            Some(t) if *t == expected => {
                self.advance();
                Ok(())
            }
            Some(t) => Err(ParseError::Unexpected {
                expected: expected_desc.to_owned(),
                got: format!("{t:?}"),
                pos: self.current_pos(),
            }),
            None => Err(ParseError::Unexpected {
                expected: expected_desc.to_owned(),
                got: "end of input".into(),
                pos: self.current_pos(),
            }),
        }
    }

    fn eat(&mut self, tok: &Token) -> bool {
        if self.peek() == Some(tok) {
            self.advance();
            true
        } else {
            false
        }
    }

    // ─── Grammar productions ─────────────────────────────────────────────────

    fn parse_query(&mut self) -> Result<Query, ParseError> {
        self.expect(Token::Find, "FIND")?;

        match self.peek() {
            Some(Token::Shortest) => {
                self.advance();
                self.expect(Token::Path, "PATH")?;
                Ok(Query::ShortestPath(self.parse_path_query()?))
            }
            Some(Token::Blast) => {
                self.advance();
                self.expect(Token::Radius, "RADIUS")?;
                Ok(Query::BlastRadius(self.parse_blast_query()?))
            }
            _ => Ok(Query::Find(self.parse_find_query()?)),
        }
    }

    fn parse_find_query(&mut self) -> Result<FindQuery, ParseError> {
        let entity = self.parse_entity_filter()?;

        let property_filters = if self.eat(&Token::With) {
            self.parse_property_expr()?
        } else {
            Vec::new()
        };

        let mut traversals = Vec::new();
        while self.eat(&Token::That) {
            traversals.push(self.parse_traversal_step()?);
        }

        let group_by = if self.eat(&Token::Group) {
            self.expect(Token::By, "BY")?;
            let field = self.parse_ident("group field name")?;
            Some(GroupByClause { field })
        } else {
            None
        };

        let return_clause = if self.eat(&Token::Return) {
            Some(self.parse_return_clause()?)
        } else {
            None
        };

        let limit = if self.eat(&Token::Limit) {
            Some(self.parse_integer()? as usize)
        } else {
            None
        };

        Ok(FindQuery { entity, property_filters, traversals, group_by, return_clause, limit })
    }

    fn parse_path_query(&mut self) -> Result<PathQuery, ParseError> {
        self.expect(Token::From, "FROM")?;
        let from = self.parse_entity_filter()?;
        let from_filters = if self.eat(&Token::With) { self.parse_property_expr()? } else { Vec::new() };

        self.expect(Token::To, "TO")?;
        let to = self.parse_entity_filter()?;
        let to_filters = if self.eat(&Token::With) { self.parse_property_expr()? } else { Vec::new() };

        let max_depth = if self.eat(&Token::Depth) {
            Some(self.parse_integer()? as u32)
        } else {
            None
        };

        Ok(PathQuery { from, from_filters, to, to_filters, max_depth })
    }

    fn parse_blast_query(&mut self) -> Result<BlastQuery, ParseError> {
        self.expect(Token::From, "FROM")?;
        let origin = self.parse_entity_filter()?;
        let origin_filters = if self.eat(&Token::With) { self.parse_property_expr()? } else { Vec::new() };

        let max_depth = if self.eat(&Token::Depth) {
            Some(self.parse_integer()? as u32)
        } else {
            None
        };

        Ok(BlastQuery { origin, origin_filters, max_depth })
    }

    fn parse_entity_filter(&mut self) -> Result<EntityFilter, ParseError> {
        match self.peek() {
            Some(Token::Star) => {
                self.advance();
                Ok(EntityFilter::wildcard())
            }
            Some(Token::Ident(_)) => {
                if let Token::Ident(name) = self.advance().expect("just peeked").clone() {
                    Ok(EntityFilter::named(name))
                } else {
                    unreachable!()
                }
            }
            Some(t) => Err(ParseError::Unexpected {
                expected: "entity type, class name, or '*'".into(),
                got: format!("{t:?}"),
                pos: self.current_pos(),
            }),
            None => Err(ParseError::Unexpected {
                expected: "entity type, class name, or '*'".into(),
                got: "end of input".into(),
                pos: self.current_pos(),
            }),
        }
    }

    fn parse_traversal_step(&mut self) -> Result<TraversalStep, ParseError> {
        let negated = self.eat(&Token::Bang);
        let verb = self.parse_verb()?;
        let target = self.parse_entity_filter()?;
        let property_filters =
            if self.eat(&Token::With) { self.parse_property_expr()? } else { Vec::new() };

        Ok(TraversalStep { negated, verb, target, property_filters })
    }

    fn parse_verb(&mut self) -> Result<Verb, ParseError> {
        let pos = self.current_pos();
        match self.advance() {
            Some(Token::Has) => Ok(Verb::Has),
            Some(Token::Is) => Ok(Verb::Is),
            Some(Token::Assigned) => Ok(Verb::Assigned),
            Some(Token::Allows) => Ok(Verb::Allows),
            Some(Token::Uses) => Ok(Verb::Uses),
            Some(Token::Contains) => Ok(Verb::Contains),
            Some(Token::Manages) => Ok(Verb::Manages),
            Some(Token::Connects) => Ok(Verb::Connects),
            Some(Token::Protects) => Ok(Verb::Protects),
            Some(Token::Exploits) => Ok(Verb::Exploits),
            Some(Token::Trusts) => Ok(Verb::Trusts),
            Some(Token::Scans) => Ok(Verb::Scans),
            Some(Token::Relates) => {
                self.expect(Token::To, "TO (after RELATES)")?;
                Ok(Verb::RelatesTo)
            }
            Some(t) => Err(ParseError::UnknownVerb { verb: format!("{t:?}"), pos }),
            None => Err(ParseError::Unexpected {
                expected: "verb (HAS, ASSIGNED, ...)".into(),
                got: "end of input".into(),
                pos,
            }),
        }
    }

    /// Parse a property expression: `cond_or (AND cond_or)*`.
    ///
    /// Each `cond_or` may itself be a disjunction: `cond (OR cond)*`.
    /// The outer list is conjunction (all must hold); `PropertyCondition::Or`
    /// wraps a disjunction (at least one must hold).
    fn parse_property_expr(&mut self) -> Result<Vec<PropertyCondition>, ParseError> {
        let mut conditions = vec![self.parse_property_or()?];
        while self.eat(&Token::And) {
            conditions.push(self.parse_property_or()?);
        }
        Ok(conditions)
    }

    /// Parse an OR-joined group: `cond (OR cond)*`.
    ///
    /// Returns a single `PropertyCondition`. If only one sub-condition is
    /// parsed, it is returned directly (no wrapping). If multiple are parsed,
    /// they are wrapped in `PropertyCondition::Or`.
    fn parse_property_or(&mut self) -> Result<PropertyCondition, ParseError> {
        let first = self.parse_property_cond()?;
        if self.peek() != Some(&Token::Or) {
            return Ok(first);
        }
        let mut arms = vec![first];
        while self.eat(&Token::Or) {
            arms.push(self.parse_property_cond()?);
        }
        Ok(PropertyCondition::Or(arms))
    }

    fn parse_property_cond(&mut self) -> Result<PropertyCondition, ParseError> {
        // NOT condition
        if self.eat(&Token::Not) {
            let inner = self.parse_property_cond()?;
            return Ok(PropertyCondition::Not(Box::new(inner)));
        }

        // identifier op value | identifier EXISTS | identifier IN (...)
        let key = self.parse_ident("property key")?;
        let pos = self.current_pos();

        match self.peek() {
            Some(Token::Eq) => {
                self.advance();
                Ok(PropertyCondition::Eq(key, self.parse_value()?))
            }
            Some(Token::Ne) => {
                self.advance();
                Ok(PropertyCondition::Ne(key, self.parse_value()?))
            }
            Some(Token::Lt) => {
                self.advance();
                Ok(PropertyCondition::Lt(key, self.parse_value()?))
            }
            Some(Token::Lte) => {
                self.advance();
                Ok(PropertyCondition::Lte(key, self.parse_value()?))
            }
            Some(Token::Gt) => {
                self.advance();
                Ok(PropertyCondition::Gt(key, self.parse_value()?))
            }
            Some(Token::Gte) => {
                self.advance();
                Ok(PropertyCondition::Gte(key, self.parse_value()?))
            }
            Some(Token::In) => {
                self.advance();
                self.expect(Token::LParen, "(")?;
                let mut vals = vec![self.parse_value()?];
                while self.eat(&Token::Comma) {
                    vals.push(self.parse_value()?);
                }
                self.expect(Token::RParen, ")")?;
                Ok(PropertyCondition::In(key, vals))
            }
            Some(Token::Like) => {
                self.advance();
                match self.advance() {
                    Some(Token::StringLit(s)) => Ok(PropertyCondition::Like(key, s.clone())),
                    Some(t) => Err(ParseError::Unexpected {
                        expected: "string literal after LIKE".into(),
                        got: format!("{t:?}"),
                        pos,
                    }),
                    None => Err(ParseError::Unexpected {
                        expected: "string literal after LIKE".into(),
                        got: "end of input".into(),
                        pos,
                    }),
                }
            }
            Some(Token::Exists) => {
                self.advance();
                Ok(PropertyCondition::Exists(key))
            }
            Some(t) => Err(ParseError::Unexpected {
                expected: "comparison operator, IN, LIKE, or EXISTS".into(),
                got: format!("{t:?}"),
                pos,
            }),
            None => Err(ParseError::Unexpected {
                expected: "comparison operator".into(),
                got: "end of input".into(),
                pos,
            }),
        }
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        let pos = self.current_pos();
        match self.advance() {
            Some(Token::StringLit(s)) => Ok(Value::from(s.as_str())),
            Some(Token::Integer(n)) => Ok(Value::Int(*n)),
            Some(Token::Float(f)) => Ok(Value::Float(*f)),
            Some(Token::True) => Ok(Value::Bool(true)),
            Some(Token::False) => Ok(Value::Bool(false)),
            Some(Token::Null) => Ok(Value::Null),
            Some(t) => Err(ParseError::Unexpected {
                expected: "value (string, integer, float, true, false, null)".into(),
                got: format!("{t:?}"),
                pos,
            }),
            None => Err(ParseError::Unexpected {
                expected: "value".into(),
                got: "end of input".into(),
                pos,
            }),
        }
    }

    fn parse_return_clause(&mut self) -> Result<ReturnClause, ParseError> {
        if self.eat(&Token::Count) {
            return Ok(ReturnClause::Count);
        }
        let mut fields = vec![self.parse_ident("field name")?];
        while self.eat(&Token::Comma) {
            fields.push(self.parse_ident("field name")?);
        }
        Ok(ReturnClause::Fields(fields))
    }

    fn parse_integer(&mut self) -> Result<i64, ParseError> {
        let pos = self.current_pos();
        match self.advance() {
            Some(Token::Integer(n)) => Ok(*n),
            Some(t) => Err(ParseError::Unexpected {
                expected: "integer".into(),
                got: format!("{t:?}"),
                pos,
            }),
            None => Err(ParseError::ExpectedInteger { pos }),
        }
    }

    fn parse_ident(&mut self, desc: &str) -> Result<String, ParseError> {
        let pos = self.current_pos();
        match self.advance() {
            Some(Token::Ident(s)) => Ok(s.clone()),
            Some(t) => Err(ParseError::Unexpected {
                expected: desc.to_owned(),
                got: format!("{t:?}"),
                pos,
            }),
            None => Err(ParseError::Unexpected {
                expected: desc.to_owned(),
                got: "end of input".into(),
                pos,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Query;

    #[test]
    fn parse_simple_find() {
        let q = parse("FIND host").unwrap();
        assert!(matches!(q, Query::Find(_)));
        if let Query::Find(fq) = q {
            assert_eq!(fq.entity.name.as_deref(), Some("host"));
            assert!(fq.property_filters.is_empty());
            assert!(fq.traversals.is_empty());
        }
    }

    #[test]
    fn parse_find_with_property() {
        let q = parse("FIND host WITH state = 'running'").unwrap();
        if let Query::Find(fq) = q {
            assert_eq!(fq.property_filters.len(), 1);
            assert!(matches!(fq.property_filters[0], PropertyCondition::Eq(ref k, ref v)
                if k == "state" && v.as_str() == Some("running")));
        }
    }

    #[test]
    fn parse_find_with_two_props() {
        let q = parse("FIND host WITH state = 'running' AND active = true").unwrap();
        if let Query::Find(fq) = q {
            assert_eq!(fq.property_filters.len(), 2);
        }
    }

    #[test]
    fn parse_traversal() {
        let q = parse("FIND User THAT ASSIGNED aws_iam_role").unwrap();
        if let Query::Find(fq) = q {
            assert_eq!(fq.traversals.len(), 1);
            assert_eq!(fq.traversals[0].verb, Verb::Assigned);
            assert!(!fq.traversals[0].negated);
            assert_eq!(fq.traversals[0].target.name.as_deref(), Some("aws_iam_role"));
        }
    }

    #[test]
    fn parse_negated_traversal() {
        let q = parse("FIND host THAT !PROTECTS edr_agent").unwrap();
        if let Query::Find(fq) = q {
            assert_eq!(fq.traversals.len(), 1);
            assert!(fq.traversals[0].negated);
            assert_eq!(fq.traversals[0].verb, Verb::Protects);
        }
    }

    #[test]
    fn parse_wildcard() {
        let q = parse("FIND *").unwrap();
        if let Query::Find(fq) = q {
            assert!(fq.entity.name.is_none());
        }
    }

    #[test]
    fn parse_return_count() {
        let q = parse("FIND host RETURN COUNT").unwrap();
        if let Query::Find(fq) = q {
            assert!(matches!(fq.return_clause, Some(ReturnClause::Count)));
        }
    }

    #[test]
    fn parse_return_fields() {
        let q = parse("FIND host RETURN display_name, region").unwrap();
        if let Query::Find(fq) = q {
            assert!(matches!(fq.return_clause, Some(ReturnClause::Fields(ref fs)) if fs.len() == 2));
        }
    }

    #[test]
    fn parse_limit() {
        let q = parse("FIND host LIMIT 100").unwrap();
        if let Query::Find(fq) = q {
            assert_eq!(fq.limit, Some(100));
        }
    }

    #[test]
    fn parse_shortest_path() {
        let q = parse("FIND SHORTEST PATH FROM User TO aws_s3_bucket DEPTH 6").unwrap();
        assert!(matches!(q, Query::ShortestPath(_)));
        if let Query::ShortestPath(pq) = q {
            assert_eq!(pq.from.name.as_deref(), Some("User"));
            assert_eq!(pq.to.name.as_deref(), Some("aws_s3_bucket"));
            assert_eq!(pq.max_depth, Some(6));
        }
    }

    #[test]
    fn parse_blast_radius() {
        let q = parse("FIND BLAST RADIUS FROM aws_ec2_instance DEPTH 4").unwrap();
        assert!(matches!(q, Query::BlastRadius(_)));
        if let Query::BlastRadius(bq) = q {
            assert_eq!(bq.origin.name.as_deref(), Some("aws_ec2_instance"));
            assert_eq!(bq.max_depth, Some(4));
        }
    }

    #[test]
    fn parse_in_condition() {
        let q = parse("FIND host WITH state IN ('running', 'starting')").unwrap();
        if let Query::Find(fq) = q {
            assert!(matches!(fq.property_filters[0], PropertyCondition::In(_, _)));
        }
    }

    #[test]
    fn parse_exists_condition() {
        let q = parse("FIND host WITH tag EXISTS").unwrap();
        if let Query::Find(fq) = q {
            assert!(matches!(fq.property_filters[0], PropertyCondition::Exists(_)));
        }
    }

    #[test]
    fn parse_not_condition() {
        let q = parse("FIND host WITH NOT active = false").unwrap();
        if let Query::Find(fq) = q {
            assert!(matches!(fq.property_filters[0], PropertyCondition::Not(_)));
        }
    }

    #[test]
    fn parse_relates_to() {
        let q = parse("FIND host THAT RELATES TO service").unwrap();
        if let Query::Find(fq) = q {
            assert_eq!(fq.traversals[0].verb, Verb::RelatesTo);
        }
    }

    #[test]
    fn parse_group_by() {
        let q = parse("FIND host GROUP BY region").unwrap();
        if let Query::Find(fq) = q {
            assert!(fq.group_by.is_some());
            assert_eq!(fq.group_by.unwrap().field, "region");
        }
    }

    #[test]
    fn parse_group_by_with_return_count() {
        let q = parse("FIND host GROUP BY region RETURN COUNT").unwrap();
        if let Query::Find(fq) = q {
            assert!(fq.group_by.is_some());
            assert!(matches!(fq.return_clause, Some(ReturnClause::Count)));
        }
    }

    #[test]
    fn parse_or_condition() {
        let q = parse("FIND host WITH state = 'running' OR state = 'starting'").unwrap();
        if let Query::Find(fq) = q {
            assert_eq!(fq.property_filters.len(), 1);
            assert!(matches!(fq.property_filters[0], PropertyCondition::Or(_)));
            if let PropertyCondition::Or(arms) = &fq.property_filters[0] {
                assert_eq!(arms.len(), 2);
            }
        }
    }

    #[test]
    fn parse_or_combined_with_and() {
        // state = 'running' OR state = 'starting' AND region = 'us-east-1'
        // Parsed as: [Or(state=running, state=starting), Eq(region, us-east-1)]
        let q = parse("FIND host WITH state = 'running' OR state = 'starting' AND region = 'us-east-1'").unwrap();
        if let Query::Find(fq) = q {
            assert_eq!(fq.property_filters.len(), 2, "AND joins two top-level conditions");
            assert!(matches!(fq.property_filters[0], PropertyCondition::Or(_)));
            assert!(matches!(fq.property_filters[1], PropertyCondition::Eq(_, _)));
        }
    }

    #[test]
    fn parse_error_unknown_verb() {
        assert!(parse("FIND host THAT JUMPS service").is_err());
    }

    #[test]
    fn parse_error_missing_to_in_path() {
        assert!(parse("FIND SHORTEST PATH FROM User aws_s3_bucket").is_err());
    }
}
