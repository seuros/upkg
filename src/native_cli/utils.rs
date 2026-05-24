use console::style;

pub use crate::package_ref::normalize_formula_name;

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
