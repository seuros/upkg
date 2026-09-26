use crate::core::progress::{InstallProgress, ProgressCallback};
use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::native_cli::utils::{explain_install_failure, normalize_formula_name};
use crate::package_ref::{is_cask_name, normalize_app_name};

pub async fn execute(
    installer: &mut crate::core::installer::install::Installer,
    formulas: Vec<String>,
    no_link: bool,
    build_from_source: bool,
    package_kind: crate::api::PackageKindHint,
) -> Result<(), crate::types::Error> {
    let start = Instant::now();
    println!(
        "{} Installing {}...",
        style("==>").cyan().bold(),
        style(formulas.join(", ")).bold()
    );

    let mut formula_names: Vec<(String, String)> = Vec::new();
    let mut cask_names: Vec<String> = Vec::new();
    for formula in &formulas {
        let result = match package_kind {
            crate::api::PackageKindHint::Auto => normalize_formula_name(formula),
            crate::api::PackageKindHint::App => normalize_app_name(formula),
        };
        match result {
            Ok(name) => {
                if is_cask_name(&name) {
                    cask_names.push(name);
                } else {
                    formula_names.push((formula.clone(), name));
                }
            }
            Err(e) => {
                explain_install_failure(formula, &e);
                return Err(e);
            }
        }
    }

    let mut installed_count = 0usize;

    if package_kind == crate::api::PackageKindHint::Auto && !formula_names.is_empty() {
        let targets = installer
            .resolve_auto_install_targets(&formula_names)
            .await
            .map_err(|e| report_execute_error(e, &formula_names, &formulas))?;

        for (original, cask_name) in targets.casks {
            println!(
                "{} {} is a cask; installing as app",
                style("==>").cyan().bold(),
                style(original).bold()
            );
            cask_names.push(cask_name);
        }

        formula_names = targets.formulas;
    }

    let normalized_names: Vec<String> = formula_names
        .iter()
        .map(|(_, normalized)| normalized.clone())
        .collect();

    if !normalized_names.is_empty() {
        let plan = installer
            .plan_with_options(&normalized_names, build_from_source)
            .await
            .map_err(|e| report_execute_error(e, &formula_names, &formulas))?;

        println!(
            "{} Resolving dependencies ({} packages)...",
            style("==>").cyan().bold(),
            plan.items.len()
        );
        for item in &plan.items {
            println!(
                "    {} {}",
                style(&item.formula.name).green(),
                style(&item.formula.versions.stable).dim()
            );
        }

        println!(
            "{} Downloading and installing formulas...",
            style("==>").cyan().bold()
        );

        let (bars, progress_callback) = progress_bars();

        let result_val = installer
            .execute_with_progress(plan, !no_link, Some(progress_callback))
            .await;
        finish_bars(&bars);

        let result = result_val.map_err(|e| report_execute_error(e, &formula_names, &formulas))?;
        installed_count += result.installed;
    }

    if !cask_names.is_empty() {
        println!(
            "{} Installing casks ({} packages)...",
            style("==>").cyan().bold(),
            cask_names.len()
        );
        let result = installer.install_casks(&cask_names, !no_link).await?;
        installed_count += result.installed;
    }

    let elapsed = start.elapsed();
    println!();
    println!(
        "{} Installed {} packages in {:.2}s",
        style("==>").cyan().bold(),
        style(installed_count).green().bold(),
        elapsed.as_secs_f64()
    );

    Ok(())
}

type ProgressBars = Arc<Mutex<HashMap<String, ProgressBar>>>;

fn set_message(
    bars: &HashMap<String, ProgressBar>,
    name: &str,
    message: impl Into<std::borrow::Cow<'static, str>>,
) {
    if let Some(pb) = bars.get(name) {
        pb.set_message(message);
    }
}

fn spin(pb: &ProgressBar, style: &ProgressStyle, message: &'static str) {
    pb.set_style(style.clone());
    pb.set_message(message);
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
}

pub(crate) fn progress_bars() -> (ProgressBars, Arc<ProgressCallback>) {
    let multi = MultiProgress::new();
    let bars: Arc<Mutex<HashMap<String, ProgressBar>>> = Arc::new(Mutex::new(HashMap::new()));

    let download_style = ProgressStyle::default_bar()
        .template("    {prefix:<16} {bar:25.cyan/dim} {bytes:>10}/{total_bytes:<10} {eta:>6}")
        .unwrap()
        .progress_chars("━━╸");

    let spinner_style = ProgressStyle::default_spinner()
        .template("    {prefix:<16} {spinner:.cyan} {msg}")
        .unwrap()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

    let done_style = ProgressStyle::default_spinner()
        .template("    {prefix:<16} {msg}")
        .unwrap();

    let bars_clone = bars.clone();
    let multi_clone = multi.clone();
    let download_style_clone = download_style.clone();
    let spinner_style_clone = spinner_style.clone();
    let done_style_clone = done_style.clone();

    let progress_callback: Arc<ProgressCallback> = Arc::new(Box::new(move |event| {
        let mut bars = bars_clone.lock().unwrap();
        match event {
            InstallProgress::DownloadStarted { name, total_bytes } => {
                let pb = if let Some(total) = total_bytes {
                    let pb = multi_clone.add(ProgressBar::new(total));
                    pb.set_style(download_style_clone.clone());
                    pb
                } else {
                    let pb = multi_clone.add(ProgressBar::new_spinner());
                    spin(&pb, &spinner_style_clone, "downloading...");
                    pb
                };
                pb.set_prefix(name.clone());
                bars.insert(name, pb);
            }
            InstallProgress::DownloadProgress {
                name,
                downloaded,
                total_bytes,
            } => {
                if let Some(pb) = bars.get(&name)
                    && total_bytes.is_some()
                {
                    pb.set_position(downloaded);
                }
            }
            InstallProgress::DownloadCompleted { name, total_bytes } => {
                if let Some(pb) = bars.get(&name) {
                    if total_bytes > 0 {
                        pb.set_position(total_bytes);
                    }
                    spin(pb, &spinner_style_clone, "unpacking...");
                }
            }
            InstallProgress::UnpackStarted { name } => set_message(&bars, &name, "unpacking..."),
            InstallProgress::UnpackCompleted { name } => set_message(&bars, &name, "unpacked"),
            InstallProgress::LinkStarted { name } => set_message(&bars, &name, "linking..."),
            InstallProgress::LinkCompleted { name } => set_message(&bars, &name, "linked"),
            InstallProgress::LinkSkipped { name, reason } => {
                set_message(&bars, &name, format!("keg-only ({reason})"))
            }
            InstallProgress::InstallCompleted { name } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_style(done_style_clone.clone());
                    pb.set_message(format!("{} installed", style("✓").green()));
                    pb.finish();
                }
            }
        }
    }));

    (bars, progress_callback)
}

pub(crate) fn finish_bars(bars: &ProgressBars) {
    for pb in bars.lock().unwrap().values() {
        if !pb.is_finished() {
            pb.finish();
        }
    }
}

/// Prints the explanation for a failed plan execution and hands the error back.
pub(crate) fn report_execute_error(
    error: crate::types::Error,
    formula_names: &[(String, String)],
    requested: &[String],
) -> crate::types::Error {
    match &error {
        crate::types::Error::LinkConflict { conflicts } => {
            eprintln!();
            eprintln!(
                "{} The link step did not complete successfully.",
                style("Error:").red().bold()
            );
            eprintln!("The formula was installed, but is not symlinked into the prefix.");
            eprintln!();
            eprintln!("Possible conflicting files:");
            for c in conflicts {
                if let Some(ref owner) = c.owned_by {
                    eprintln!(
                        "  {} (symlink belonging to {})",
                        c.path.display(),
                        style(owner).yellow()
                    );
                } else {
                    eprintln!("  {}", c.path.display());
                }
            }
            eprintln!();
        }
        _ => {
            let formula = failure_context_for_error(&error, formula_names, requested);
            explain_install_failure(&formula, &error);
        }
    }
    error
}

pub(crate) fn failure_context_for_error(
    error: &crate::types::Error,
    formula_names: &[(String, String)],
    requested: &[String],
) -> String {
    if let Some(error_name) = error_formula_name(error) {
        if let Some((original, _)) = formula_names
            .iter()
            .find(|(_, normalized)| normalized == error_name)
        {
            return original.clone();
        }
        return error_name.to_string();
    }

    requested.join(", ")
}

fn error_formula_name(error: &crate::types::Error) -> Option<&str> {
    match error {
        crate::types::Error::UnsupportedBottle { name }
        | crate::types::Error::MissingFormula { name }
        | crate::types::Error::UnsupportedTap { name }
        | crate::types::Error::UnsupportedFormula { name, .. }
        | crate::types::Error::NotInstalled { name } => Some(name),
        _ => None,
    }
}
