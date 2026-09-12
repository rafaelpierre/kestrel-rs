//! Bounded portable query parsing and matching against provider result metadata.
//!
//! This is lexical constraint checking, not a page-content or relevance claim.
use crate::{SearchResult, search::KestrelError};
use url::Url;

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum, PartialEq, Eq)]
pub enum QuerySyntax {
    /// Check portable constraints against titles/snippets before accepting results.
    #[default]
    Portable,
    /// Pass provider-specific syntax through without portable metadata checks.
    Native,
}

#[derive(Debug)]
enum Expr {
    Text(String),
    Site(String),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

#[derive(Debug, PartialEq)]
enum Token {
    Text(String),
    Word(String),
    And,
    Or,
    Not,
    Open,
    Close,
}

#[derive(Debug)]
pub(crate) struct QueryPlan(Option<Expr>);

fn invalid(message: &str) -> KestrelError {
    KestrelError::InvalidRequest(format!(
        "Invalid portable query: {message}. Use --query-syntax native for provider-specific syntax"
    ))
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn lex(query: &str) -> Result<Vec<Token>, KestrelError> {
    if query.len() > 8192 {
        return Err(invalid("maximum query size is 8192 bytes"));
    }
    let mut chars = query.chars().peekable();
    let mut tokens = Vec::new();
    while let Some(c) = chars.next() {
        let token = match c {
            c if c.is_whitespace() => continue,
            '(' => Token::Open,
            ')' => Token::Close,
            '-' => Token::Not,
            '"' => {
                let mut value = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' => match chars.next() {
                            Some(c @ ('"' | '\\')) => value.push(c),
                            _ => {
                                return Err(invalid(
                                    "only escaped quotes and backslashes are supported",
                                ));
                            }
                        },
                        _ => value.push(c),
                    }
                }
                if !closed {
                    return Err(invalid("unclosed quoted phrase"));
                }
                let value = normalize(&value);
                if value.is_empty() {
                    return Err(invalid("empty quoted phrase"));
                }
                Token::Text(value)
            }
            _ => {
                let mut value = String::from(c);
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || matches!(next, '(' | ')' | '"') {
                        break;
                    }
                    value.push(chars.next().unwrap());
                }
                match value.as_str() {
                    "AND" => Token::And,
                    "OR" => Token::Or,
                    "NOT" => Token::Not,
                    _ => Token::Word(value),
                }
            }
        };
        tokens.push(token);
        if tokens.len() > 128 {
            return Err(invalid("maximum query length is 128 tokens"));
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: std::collections::VecDeque<Token>,
}

impl Parser {
    fn or(&mut self, depth: usize) -> Result<Expr, KestrelError> {
        let mut left = self.and(depth)?;
        while self.tokens.front() == Some(&Token::Or) {
            self.tokens.pop_front();
            left = Expr::Or(Box::new(left), Box::new(self.and(depth)?));
        }
        Ok(left)
    }

    fn and(&mut self, depth: usize) -> Result<Expr, KestrelError> {
        let mut left = self.atom(depth)?;
        while let Some(token) = self.tokens.front() {
            if matches!(token, Token::Close | Token::Or) {
                break;
            }
            if *token == Token::And {
                self.tokens.pop_front();
            }
            left = Expr::And(Box::new(left), Box::new(self.atom(depth)?));
        }
        Ok(left)
    }

    fn atom(&mut self, depth: usize) -> Result<Expr, KestrelError> {
        if depth > 32 {
            return Err(invalid("maximum nesting depth is 32"));
        }
        match self.tokens.pop_front() {
            Some(Token::Not) => Ok(Expr::Not(Box::new(self.atom(depth + 1)?))),
            Some(Token::Open) => {
                let inner = self.or(depth + 1)?;
                if self.tokens.pop_front() != Some(Token::Close) {
                    return Err(invalid("missing closing parenthesis"));
                }
                Ok(inner)
            }
            Some(Token::Text(value)) => Ok(Expr::Text(value)),
            Some(Token::Word(value)) => {
                if let Some((operator, domain)) = value.split_once(':') {
                    if !operator.eq_ignore_ascii_case("site") {
                        return Err(invalid(
                            "unsupported operator (portable mode supports site:hostname)",
                        ));
                    }
                    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
                    if !domain.contains('.')
                        || !domain.split('.').all(|label| {
                            !label.is_empty()
                                && !label.starts_with('-')
                                && !label.ends_with('-')
                                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                        })
                    {
                        return Err(invalid(
                            "site: requires a hostname, without a scheme, path or wildcard",
                        ));
                    }
                    Ok(Expr::Site(domain))
                } else if value.contains(['\\', '|', '*']) {
                    Err(invalid("unsupported escape, pipe or wildcard"))
                } else {
                    Ok(Expr::Text(normalize(&value)))
                }
            }
            _ => Err(invalid("expected a term, phrase or grouped expression")),
        }
    }
}

impl QueryPlan {
    pub(crate) fn parse(query: &str, syntax: QuerySyntax) -> Result<Self, KestrelError> {
        if syntax == QuerySyntax::Native {
            return Ok(Self(None));
        }
        let mut parser = Parser {
            tokens: lex(query)?.into(),
        };
        let expression = parser.or(0)?;
        if !parser.tokens.is_empty() {
            return Err(invalid("unexpected closing parenthesis"));
        }
        Ok(Self(Some(expression)))
    }

    pub(crate) fn is_native(&self) -> bool {
        self.0.is_none()
    }

    pub(crate) fn matches(&self, result: &SearchResult) -> bool {
        let Some(expression) = &self.0 else {
            return true;
        };
        let title = normalize(&result.title);
        let snippet = normalize(&result.snippet);
        let host = Url::parse(&result.url).ok().and_then(|url| {
            url.host_str()
                .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
        });
        expression.matches(&title, &snippet, host.as_deref())
    }
}

fn contains(text: &str, needle: &str) -> bool {
    // Keep programming identifiers such as C++, C# and snake_case intact.
    let word = |c: char| c.is_alphanumeric() || matches!(c, '_' | '+' | '#');
    text.match_indices(needle).any(|(start, _)| {
        let end = start + needle.len();
        text[..start].chars().next_back().is_none_or(|c| !word(c))
            && text[end..].chars().next().is_none_or(|c| !word(c))
    })
}

impl Expr {
    fn matches(&self, title: &str, snippet: &str, host: Option<&str>) -> bool {
        match self {
            Self::Text(value) => contains(title, value) || contains(snippet, value),
            Self::Site(domain) => {
                host.is_some_and(|host| host == domain || host.ends_with(&format!(".{domain}")))
            }
            Self::Not(inner) => !inner.matches(title, snippet, host),
            Self::And(a, b) => a.matches(title, snippet, host) && b.matches(title, snippet, host),
            Self::Or(a, b) => a.matches(title, snippet, host) || b.matches(title, snippet, host),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hit(title: &str, snippet: &str) -> SearchResult {
        SearchResult::parsed(
            title.into(),
            "https://docs.example.com/page".into(),
            String::new(),
            snippet.into(),
        )
    }
    fn matches(query: &str, title: &str, snippet: &str) -> bool {
        QueryPlan::parse(query, QuerySyntax::Portable)
            .unwrap()
            .matches(&hit(title, snippet))
    }
    #[test]
    fn phrase_and_conjunction_are_distinct() {
        assert!(matches(
            "\"machine learning\"",
            "MACHINE\nLearning guide",
            ""
        ));
        assert!(!matches("\"machine learning\"", "Machine", "Learning"));
        assert!(!matches(
            "\"machine learning\"",
            "machine assisted learning",
            ""
        ));
        assert!(matches("machine AND learning", "Machine", "Learning"));
        assert!(matches("machine learning", "Learning machine", ""));
        assert!(!matches("machine learning", "Machine Mart", "power tools"));
        assert!(!matches("machine", "machinery", ""));
    }
    #[test]
    fn precedence_grouping_exclusions_and_sites() {
        assert!(matches("rust OR python AND async", "Rust", ""));
        assert!(!matches("(rust OR python) AND async", "Rust", ""));
        assert!(matches("(rust OR python) async -game", "Rust async", ""));
        assert!(!matches("rust NOT game", "Rust game", ""));
        assert!(matches("site:example.com rust", "Rust", ""));
        assert!(!matches(
            "site:other.example.com OR site:evil-example.com",
            "Rust",
            ""
        ));
        assert!(matches("NOT site:evil-example.com rust", "Rust", ""));
        assert!(matches("\"site:literal\"", "site:literal", ""));
    }
    #[test]
    fn unicode_escapes_and_code_identifiers() {
        assert!(matches("C++ café 日本語", "C++ CAFÉ 日本語", ""));
        assert!(!matches("C++", "C# and C", ""));
        assert!(!matches("Rust", "Rustlang", ""));
        assert!(matches(r#""say \"hello\"""#, "say \"hello\"", ""));
        assert!(matches("and", "and", ""));
    }
    #[test]
    fn malformed_or_unsupported_queries_are_rejected_but_native_is_available() {
        for query in [
            "",
            "\"\"",
            "\"machine",
            "a AND",
            "OR a",
            "a OR OR b",
            "()",
            "(a",
            "a)",
            "-",
            "site:",
            "site:example.com/path",
            "filetype:pdf",
            "a|b",
            "a*",
            r#""bad\q""#,
        ] {
            assert!(
                QueryPlan::parse(query, QuerySyntax::Portable).is_err(),
                "{query}"
            );
            assert!(
                QueryPlan::parse(query, QuerySyntax::Native)
                    .unwrap()
                    .matches(&hit("anything", ""))
            );
        }
        for query in [
            "a ".repeat(129),
            "(".repeat(33) + "a" + &")".repeat(33),
            "a".repeat(8193),
        ] {
            assert!(QueryPlan::parse(&query, QuerySyntax::Portable).is_err());
        }
    }
    #[test]
    fn nesting_limit_accepts_32_and_rejects_33_levels() {
        for depth in [31, 32, 33] {
            for query in [
                "(".repeat(depth) + "a" + &")".repeat(depth),
                "NOT ".repeat(depth) + "a",
                "(".repeat(depth / 2)
                    + &"NOT ".repeat(depth - depth / 2)
                    + "a"
                    + &")".repeat(depth / 2),
            ] {
                let parsed = QueryPlan::parse(&query, QuerySyntax::Portable);
                if depth <= 32 {
                    assert!(parsed.is_ok(), "{query}: {parsed:?}");
                } else {
                    assert!(
                        parsed
                            .unwrap_err()
                            .to_string()
                            .contains("maximum nesting depth is 32")
                    );
                }
            }
        }
    }

    #[test]
    fn matching_uses_metadata_only() {
        let mut result = hit("Machine", "");
        result.content = Some("machine learning".into());
        result.url = "https://example.com/machine-learning".into();
        assert!(
            !QueryPlan::parse("\"machine learning\"", QuerySyntax::Portable)
                .unwrap()
                .matches(&result)
        );
    }
}
