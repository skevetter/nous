/// Sanitize a user-supplied query string for use in an `SQLite` FTS5 `MATCH` clause.
///
/// Tokens containing FTS5 operator characters are wrapped in double quotes
/// (with any internal double quotes stripped) so the engine treats them as
/// literals rather than syntax.
pub fn sanitize_fts5_query(query: &str) -> String {
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|token| {
            let stripped = token.replace('"', "");
            let upper = stripped.to_uppercase();
            if matches!(upper.as_str(), "AND" | "OR" | "NOT" | "NEAR")
                || stripped.chars().any(|c| {
                    matches!(
                        c,
                        '-' | ':' | '.' | '/' | '\\' | '@' | '#' | '!' | '+' | '(' | ')' | '*'
                            | '^' | '?'
                    )
                })
            {
                format!("\"{stripped}\"")
            } else {
                stripped
            }
        })
        .collect();

    tokens.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_query_unchanged() {
        assert_eq!(sanitize_fts5_query("simple query"), "simple query");
    }

    #[test]
    fn hyphenated_token_quoted() {
        assert_eq!(sanitize_fts5_query("INI-076"), "\"INI-076\"");
    }

    #[test]
    fn at_sign_token_quoted() {
        assert_eq!(
            sanitize_fts5_query("fix auth@service"),
            "fix \"auth@service\""
        );
    }

    #[test]
    fn parentheses_quoted() {
        assert_eq!(sanitize_fts5_query("foo(bar)"), "\"foo(bar)\"");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(sanitize_fts5_query(""), "");
        assert_eq!(sanitize_fts5_query("   "), "");
    }

    #[test]
    fn embedded_quotes_stripped() {
        assert_eq!(sanitize_fts5_query("say \"hello\""), "say hello");
    }

    #[test]
    fn asterisk_quoted() {
        assert_eq!(sanitize_fts5_query("test*"), "\"test*\"");
    }

    #[test]
    fn caret_quoted() {
        assert_eq!(sanitize_fts5_query("^boost"), "\"^boost\"");
    }

    #[test]
    fn colon_quoted() {
        assert_eq!(sanitize_fts5_query("field:value"), "\"field:value\"");
    }

    #[test]
    fn mixed_safe_and_special() {
        assert_eq!(
            sanitize_fts5_query("normal token-with-hyphens also safe"),
            "normal \"token-with-hyphens\" also safe"
        );
    }

    #[test]
    fn slash_and_backslash_quoted() {
        assert_eq!(sanitize_fts5_query("path/to/file"), "\"path/to/file\"");
        assert_eq!(sanitize_fts5_query("path\\to"), "\"path\\to\"");
    }

    #[test]
    fn exclamation_and_hash_quoted() {
        assert_eq!(sanitize_fts5_query("!important"), "\"!important\"");
        assert_eq!(sanitize_fts5_query("#channel"), "\"#channel\"");
    }

    #[test]
    fn plus_quoted() {
        assert_eq!(sanitize_fts5_query("c++"), "\"c++\"");
    }

    #[test]
    fn special_token_with_embedded_quotes() {
        assert_eq!(sanitize_fts5_query("pre-\"fix\""), "\"pre-fix\"");
    }

    #[test]
    fn and_operator_quoted() {
        assert_eq!(sanitize_fts5_query("foo AND bar"), "foo \"AND\" bar");
    }

    #[test]
    fn or_operator_quoted() {
        assert_eq!(sanitize_fts5_query("foo OR bar"), "foo \"OR\" bar");
    }

    #[test]
    fn not_operator_quoted() {
        assert_eq!(sanitize_fts5_query("NOT foo"), "\"NOT\" foo");
    }

    #[test]
    fn near_operator_quoted() {
        assert_eq!(sanitize_fts5_query("foo NEAR bar"), "foo \"NEAR\" bar");
    }

    #[test]
    fn case_insensitive_operators() {
        assert_eq!(sanitize_fts5_query("foo and bar"), "foo \"and\" bar");
        assert_eq!(sanitize_fts5_query("foo Or bar"), "foo \"Or\" bar");
        assert_eq!(sanitize_fts5_query("not foo"), "\"not\" foo");
    }

    #[test]
    fn question_mark_quoted() {
        assert_eq!(sanitize_fts5_query("what?"), "\"what?\"");
    }

    #[test]
    fn operators_within_words_not_quoted() {
        assert_eq!(sanitize_fts5_query("android"), "android");
        assert_eq!(sanitize_fts5_query("notification"), "notification");
        assert_eq!(sanitize_fts5_query("fortune"), "fortune");
    }
}
