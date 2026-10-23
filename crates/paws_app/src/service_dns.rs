use crate::AppError;

/// Preserve the former service-policy input rules at the application boundary.
pub(crate) fn normalize_domains(text: &str) -> Result<Vec<String>, AppError> {
    let domains = text
        .split(|character: char| character == ',' || character.is_whitespace())
        .filter(|domain| !domain.is_empty())
        .map(|domain| {
            domain
                .trim_start_matches("+.")
                .trim_start_matches('.')
                .to_ascii_lowercase()
        })
        .collect::<Vec<_>>();
    if domains.is_empty()
        || domains.len() > 32
        || domains.iter().any(|domain| {
            !domain.contains('.')
                || domain.len() > 253
                || !domain.chars().all(|character| {
                    character.is_ascii_alphanumeric() || character == '.' || character == '-'
                })
        })
    {
        return Err(AppError::InvalidRequest(
            "enter at most 32 valid domains, separated by commas".into(),
        ));
    }
    Ok(domains)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_the_legacy_domain_input_and_rejects_invalid_or_unbounded_lists() {
        assert_eq!(
            normalize_domains("+.Example.COM, .example.net\nexample.org").unwrap(),
            vec!["example.com", "example.net", "example.org"]
        );
        assert!(normalize_domains("https://example.com").is_err());
        assert!(normalize_domains(" ").is_err());
        assert!(normalize_domains(&vec!["example.com"; 33].join(",")).is_err());
    }
}
