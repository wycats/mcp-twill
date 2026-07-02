use crate::{FrameworkError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandTemplate {
    pub raw: String,
    pub tokens: Vec<TemplateToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateToken {
    Literal(String),
    Placeholder(String),
}

impl CommandTemplate {
    pub fn parse(raw: impl Into<String>) -> Result<Self> {
        let raw = raw.into();
        let argv = tokenize(&raw)?;
        if argv.is_empty() {
            return Err(FrameworkError::EmptyCommand);
        }

        let mut tokens = Vec::with_capacity(argv.len());
        for token in argv {
            reject_shell_token(&token)?;
            if token.contains("$args.") {
                if !token.starts_with("$args.") || token.matches("$args.").count() != 1 {
                    return Err(FrameworkError::PlaceholderInterpolation(token));
                }
                let name = token.trim_start_matches("$args.");
                if name.is_empty() || !name.chars().all(is_arg_name_char) {
                    return Err(FrameworkError::InvalidPlaceholder(token));
                }
                tokens.push(TemplateToken::Placeholder(name.to_string()));
            } else if token.starts_with('$') {
                return Err(FrameworkError::InvalidPlaceholder(token));
            } else {
                tokens.push(TemplateToken::Literal(token));
            }
        }

        Ok(Self { raw, tokens })
    }

    pub fn literal_prefix(&self) -> Vec<&str> {
        self.tokens
            .iter()
            .map_while(|token| match token {
                TemplateToken::Literal(value) => Some(value.as_str()),
                TemplateToken::Placeholder(_) => None,
            })
            .collect()
    }

    pub fn placeholders(&self) -> Vec<&str> {
        self.tokens
            .iter()
            .filter_map(|token| match token {
                TemplateToken::Placeholder(name) => Some(name.as_str()),
                TemplateToken::Literal(_) => None,
            })
            .collect()
    }
}

fn tokenize(raw: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for ch in raw.chars() {
        match (quote, ch) {
            (Some(active), value) if value == active => quote = None,
            (None, '"' | '\'') => quote = Some(ch),
            (None, value) if value.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if quote.is_some() {
        return Err(FrameworkError::UnterminatedQuote);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn reject_shell_token(token: &str) -> Result<()> {
    const REJECTED: [&str; 13] = [
        "|", ">", "<", "2>", "2>&1", "&&", "||", ";", "$(", "`", "*", "?", "[",
    ];
    for rejected in REJECTED {
        if token.contains(rejected) {
            return Err(FrameworkError::ShellSyntax(rejected.to_string()));
        }
    }
    if token.contains(']') {
        return Err(FrameworkError::ShellSyntax("]".to_string()));
    }
    Ok(())
}

fn is_arg_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}
