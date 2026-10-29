use crate::formula::types::BottleFile;
use crate::{Error, Formula};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedBottle {
    pub tag: String,
    pub url: String,
    pub sha256: String,
}

const MACOS_CODENAMES_NEWEST_FIRST: &[&str] = &[
    "tahoe",
    "sequoia",
    "sonoma",
    "ventura",
    "monterey",
    "big_sur",
    "catalina",
    "mojave",
    "high_sierra",
];

#[cfg(target_os = "macos")]
fn current_macos_codename() -> Option<&'static str> {
    let output = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    let version = String::from_utf8_lossy(&output.stdout);
    codename_for_product_version(version.trim())
}

#[cfg(target_os = "macos")]
fn codename_for_product_version(version: &str) -> Option<&'static str> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|part| part.parse().ok());
    codename_for_version_parts(major, minor)
}

#[cfg(target_os = "macos")]
fn codename_for_version_parts(major: u32, minor: Option<u32>) -> Option<&'static str> {
    match (major, minor) {
        (10, Some(15)) => Some("catalina"),
        (10, Some(14)) => Some("mojave"),
        (10, Some(13)) => Some("high_sierra"),
        (major, _) => match major {
            26 => Some("tahoe"),
            15 => Some("sequoia"),
            14 => Some("sonoma"),
            13 => Some("ventura"),
            12 => Some("monterey"),
            11 => Some("big_sur"),
            _ => None,
        },
    }
}

pub fn compatible_codenames(current_codename: Option<&'static str>) -> Vec<&'static str> {
    let Some(codename) = current_codename else {
        return Vec::new();
    };

    let Some(pos) = MACOS_CODENAMES_NEWEST_FIRST
        .iter()
        .position(|&tag| tag == codename)
    else {
        return Vec::new();
    };

    MACOS_CODENAMES_NEWEST_FIRST[pos..].to_vec()
}

pub fn select_bottle(formula: &Formula) -> Result<SelectedBottle, Error> {
    #[cfg(target_os = "macos")]
    let macos_codename = current_macos_codename();
    #[cfg(not(target_os = "macos"))]
    let macos_codename: Option<&'static str> = None;

    select_bottle_with_codename(formula, macos_codename)
}

pub fn current_platform_bottle_candidates() -> Vec<String> {
    platform_bottle_candidates()
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn platform_bottle_candidates() -> Vec<String> {
    let Some(codename) = current_macos_codename() else {
        return Vec::new();
    };
    compatible_codenames(Some(codename))
        .into_iter()
        .map(|codename| format!("arm64_{codename}"))
        .collect()
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn platform_bottle_candidates() -> Vec<String> {
    let Some(codename) = current_macos_codename() else {
        return Vec::new();
    };
    compatible_codenames(Some(codename))
        .into_iter()
        .map(ToString::to_string)
        .collect()
}

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
)))]
fn platform_bottle_candidates() -> Vec<String> {
    Vec::new()
}

pub fn current_platform_bottle_tag() -> Option<String> {
    current_platform_bottle_candidates().into_iter().next()
}

fn selected(tag: impl Into<String>, file: &BottleFile) -> SelectedBottle {
    SelectedBottle {
        tag: tag.into(),
        url: file.url.clone(),
        sha256: file.sha256.clone(),
    }
}

fn select_bottle_with_codename(
    formula: &Formula,
    macos_codename: Option<&'static str>,
) -> Result<SelectedBottle, Error> {
    let _ = &macos_codename;

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let tags: Vec<String> = compatible_codenames(macos_codename)
            .iter()
            .map(|codename| format!("arm64_{codename}"))
            .collect();

        for tag in &tags {
            if let Some(file) = formula.bottle.stable.files.get(tag.as_str()) {
                return Ok(selected(tag.clone(), file));
            }
        }
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        for tag in compatible_codenames(macos_codename) {
            if let Some(file) = formula.bottle.stable.files.get(tag) {
                return Ok(selected(tag.to_string(), file));
            }
        }
    }

    if let Some(file) = formula.bottle.stable.files.get("all") {
        return Ok(selected("all".to_string(), file));
    }

    // Newer macOS bottles can link to system libraries unavailable on this host.
    // Let the installer build from source or fail instead of pouring one.
    Err(Error::UnsupportedBottle {
        name: formula.name.clone(),
    })
}

#[cfg(test)]
#[path = "bottle/tests.rs"]
mod tests;
