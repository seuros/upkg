use console::style;

pub fn normalize_formula_name(name: &str) -> Result<String, crate::types::Error> {
    let trimmed = name.trim();
    if let Some(token) = trimmed.strip_prefix("cask:") {
        if token.is_empty() {
            return Err(crate::types::Error::InvalidArgument {
                message: "cask token cannot be empty".to_string(),
            });
        }
        return Ok(trimmed.to_string());
    }

    if let Some((tap, formula)) = trimmed.rsplit_once('/') {
        if formula.is_empty() {
            return Err(crate::types::Error::MissingFormula {
                name: trimmed.to_string(),
            });
        }

        if tap == "homebrew/core" {
            return Ok(formula.to_string());
        }

        if tap == "homebrew/cask" {
            return Ok(format!("cask:{formula}"));
        }

        return Ok(trimmed.to_string());
    }

    Ok(trimmed.to_string())
}

pub fn explain_install_failure(formula: &str, error: &crate::types::Error) {
    eprintln!();
    eprintln!(
        "{} upkg could not install this package.",
        style("Note:").yellow().bold()
    );
    eprintln!("      Error: {}", error);
    eprintln!();

    if cfg!(target_os = "android") {
        eprintln!(
            "      {} {}",
            style(formula).yellow().bold(),
            style(
                "is not compatible with Termux - homebrew bottles are not available for Android."
            )
            .red()
            .bold()
        );
        eprintln!(
            "      {}",
            style("and cannot be installed on it.").red().bold()
        );
    } else {
        eprintln!(
            "      {}",
            style("upkg keeps Homebrew-compatible package names on macOS, but not every").yellow()
        );
        eprintln!(
            "      {}",
            style("formula, tap, bottle variant, or install path is implemented yet.").yellow()
        );
        eprintln!("      Requested package: {}", style(formula).cyan());
    }

    eprintln!();
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::normalize_formula_name;

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
}
