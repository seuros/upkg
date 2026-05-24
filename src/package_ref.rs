use crate::types::Error;

const CASK_PREFIX: &str = "cask:";

pub fn is_cask_name(name: &str) -> bool {
    name.starts_with(CASK_PREFIX)
}

pub fn cask_token(name: &str) -> Option<&str> {
    name.strip_prefix(CASK_PREFIX)
}

pub fn normalize_formula_name(name: &str) -> Result<String, Error> {
    let trimmed = name.trim();
    if let Some(token) = cask_token(trimmed) {
        if token.is_empty() {
            return Err(Error::InvalidArgument {
                message: "cask token cannot be empty".to_string(),
            });
        }
        return Ok(trimmed.to_string());
    }

    if let Some((tap, formula)) = trimmed.rsplit_once('/') {
        if formula.is_empty() {
            return Err(Error::MissingFormula {
                name: trimmed.to_string(),
            });
        }

        if tap == "homebrew/core" {
            return Ok(formula.to_string());
        }

        if tap == "homebrew/cask" {
            return Ok(cask_name(formula));
        }

        return Ok(trimmed.to_string());
    }

    Ok(trimmed.to_string())
}

pub fn normalize_app_name(name: &str) -> Result<String, Error> {
    let normalized = normalize_formula_name(name)?;
    if is_cask_name(&normalized) {
        return Ok(normalized);
    }
    if normalized.contains('/') {
        return Err(Error::InvalidArgument {
            message: format!("'{name}' is not a supported app reference"),
        });
    }
    Ok(cask_name(&normalized))
}

pub fn cask_name(token: &str) -> String {
    format!("{CASK_PREFIX}{token}")
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn normalize_core_tap_formula() {
        assert_eq!(
            normalize_formula_name("homebrew/core/wget").unwrap(),
            "wget".to_string()
        );
    }

    #[test]
    fn normalize_external_tap_formula_keeps_full_name() {
        assert_eq!(
            normalize_formula_name("hashicorp/tap/terraform").unwrap(),
            "hashicorp/tap/terraform".to_string()
        );
    }

    #[test]
    fn normalize_homebrew_cask_prefixes_token() {
        assert_eq!(
            normalize_formula_name("homebrew/cask/docker-desktop").unwrap(),
            "cask:docker-desktop".to_string()
        );
    }

    #[test]
    fn normalize_app_name_prefixes_plain_token() {
        assert_eq!(normalize_app_name("ghostty").unwrap(), "cask:ghostty");
    }

    #[test]
    fn normalize_app_name_rejects_non_cask_tap() {
        let err = normalize_app_name("hashicorp/tap/terraform").unwrap_err();
        assert!(matches!(err, Error::InvalidArgument { .. }));
    }
}
