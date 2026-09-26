use console::style;
use std::time::Instant;

use crate::native_cli::commands::install::{finish_bars, progress_bars, report_execute_error};
use crate::native_cli::utils::{explain_install_failure, normalize_formula_name};
use crate::package_ref::is_cask_name;

pub async fn execute(
    installer: &mut crate::core::installer::install::Installer,
    formulas: Vec<String>,
    no_link: bool,
    build_from_source: bool,
) -> Result<(), crate::types::Error> {
    let start = Instant::now();
    println!(
        "{} Reinstalling {}...",
        style("==>").cyan().bold(),
        style(formulas.join(", ")).bold()
    );

    let mut formula_names: Vec<(String, String)> = Vec::with_capacity(formulas.len());
    for formula in &formulas {
        let name = normalize_formula_name(formula).inspect_err(|e| {
            explain_install_failure(formula, e);
        })?;
        if is_cask_name(&name) {
            return Err(crate::types::Error::InvalidArgument {
                message: format!("reinstall does not support apps yet: {formula}"),
            });
        }
        formula_names.push((formula.clone(), name));
    }
    let names: Vec<String> = formula_names.iter().map(|(_, n)| n.clone()).collect();

    let plan = installer
        .plan_reinstall(&names, build_from_source)
        .await
        .map_err(|e| report_execute_error(e, &formula_names, &formulas))?;

    for item in &plan.from_store {
        println!(
            "    {} {} {}",
            style(&item.receipt.install_name).green(),
            style(&item.receipt.version).dim(),
            style("(from store)").dim()
        );
    }
    for item in &plan.fetch.items {
        println!(
            "    {} {}",
            style(&item.formula.name).green(),
            style(&item.formula.versions.stable).dim()
        );
    }

    let (bars, progress_callback) = progress_bars();
    let result = installer
        .reinstall_with_progress(plan, !no_link, Some(progress_callback))
        .await;
    finish_bars(&bars);
    let result = result.map_err(|e| report_execute_error(e, &formula_names, &formulas))?;

    println!();
    println!(
        "{} Reinstalled {} packages in {:.2}s",
        style("==>").cyan().bold(),
        style(result.installed).green().bold(),
        start.elapsed().as_secs_f64()
    );

    Ok(())
}
