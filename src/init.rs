use console::style;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub enum InitError {
    Message(String),
}

const PREFIX_MANAGED_DIRS: &[&str] = &[
    "bin",
    "sbin",
    "Cellar",
    "opt",
    "lib",
    "libexec",
    "cli-plugins",
    "include",
    "share",
    "etc",
];

pub(crate) fn managed_dirs(root: &Path, prefix: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        root.join("store"),
        root.join("cache"),
        root.join("locks"),
        root.join("db"),
    ];
    dirs.extend(PREFIX_MANAGED_DIRS.iter().map(|dir| prefix.join(dir)));
    dirs
}

fn home() -> Result<String, InitError> {
    std::env::var("HOME").map_err(|_| InitError::Message("HOME not set".to_string()))
}

pub fn needs_init(root: &Path, prefix: &Path) -> bool {
    managed_dirs(root, prefix)
        .iter()
        .any(|dir| !dir.exists() || !is_writable(dir))
}

pub fn is_writable(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let test_file = path.join(".upkg_write_test");
    match std::fs::write(&test_file, b"test") {
        Ok(_) => {
            let _ = std::fs::remove_file(&test_file);
            true
        }
        Err(_) => false,
    }
}

const MAX_PREFIX_LEN_MACOS: usize = 13;

pub fn run_init(root: &Path, prefix: &Path, no_modify_path: bool) -> Result<(), InitError> {
    if cfg!(target_os = "macos") {
        let prefix_str = prefix.to_string_lossy();
        if prefix_str.len() > MAX_PREFIX_LEN_MACOS {
            println!(
                "{} Prefix \"{}\" ({} chars) exceeds the macOS Mach-O limit of {} characters.",
                style("Warning:").yellow().bold(),
                prefix_str,
                prefix_str.len(),
                MAX_PREFIX_LEN_MACOS,
            );
            println!("         Path-sensitive packages (e.g. git, curl) will fail to install.");
            println!(
                "         Consider a shorter prefix, e.g.: {}",
                style("upkg init <root> /opt/homebrew").cyan(),
            );
            println!();
        }
    }

    println!("{} Initializing upkg...", style("==>").cyan().bold());

    let upkg_dir = match std::env::var("UPKG_DIR") {
        Ok(dir) => dir,
        Err(_) => {
            let home = home()?;
            format!("{}/.upkg", home)
        }
    };
    let upkg_bin = format!("{}/bin", upkg_dir);

    let root_existed = root.exists();
    let prefix_existed = prefix.exists();
    let dirs_to_create = managed_dirs(root, prefix);

    let need_sudo = dirs_to_create.iter().any(|d| {
        if d.exists() {
            !is_writable(d)
        } else {
            let mut ancestor = d.parent();
            while let Some(p) = ancestor {
                if p.exists() {
                    return !is_writable(p);
                }
                ancestor = p.parent();
            }
            true
        }
    });

    if need_sudo {
        println!(
            "{}",
            style("    Creating directories (requires elevated privileges)...").dim()
        );

        let user = Command::new("whoami")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "root".to_string()));

        let mut chown_targets = Vec::new();
        if !root_existed {
            chown_targets.push(root.to_path_buf());
        }
        if prefix != root && !prefix_existed {
            chown_targets.push(prefix.to_path_buf());
        }
        chown_targets.extend(dirs_to_create.iter().cloned());

        let mkdir_cmds: Vec<String> = dirs_to_create
            .iter()
            .map(|d| Ok(format!("mkdir -p {}", shell_quote_path(d)?)))
            .collect::<Result<_, InitError>>()?;
        let user_arg = shell_quote(&user)?;
        let chown_cmds: Vec<String> = chown_targets
            .iter()
            .map(|t| Ok(format!("chown -R {user_arg} {}", shell_quote_path(t)?)))
            .collect::<Result<_, InitError>>()?;

        let all_cmds = [mkdir_cmds, chown_cmds].concat().join(" && ");

        let status =
            crate::privilege_macos::escalate_privilege(&all_cmds).map_err(InitError::Message)?;
        if !status {
            return Err(InitError::Message(
                "Failed to create directories: privilege escalation denied".to_string(),
            ));
        }
    } else {
        for dir in &dirs_to_create {
            std::fs::create_dir_all(dir).map_err(|e| {
                InitError::Message(format!("Failed to create {}: {}", dir.display(), e))
            })?;
        }
    }

    add_to_path(prefix, &upkg_dir, &upkg_bin, root, no_modify_path)?;

    println!("{} Initialization complete!", style("==>").cyan().bold());

    Ok(())
}

fn shell_quote_path(path: &Path) -> Result<String, InitError> {
    let value = path.to_str().ok_or_else(|| {
        InitError::Message(format!("Path is not valid UTF-8: {}", path.display()))
    })?;
    shell_quote(value)
}

fn shell_quote(value: &str) -> Result<String, InitError> {
    shlex::try_quote(value)
        .map(|quoted| quoted.into_owned())
        .map_err(|e| InitError::Message(format!("Value cannot be shell quoted: {e}")))
}

const UPKG_BLOCK_START: &str = "# >>> upkg >>>";
const UPKG_BLOCK_END: &str = "# <<< upkg <<<";
const OLD_UPKG_BLOCK_START: &str = "# >>> upkg-native >>>";
const OLD_UPKG_BLOCK_END: &str = "# <<< upkg-native <<<";

fn managed_block_range(
    existing: &str,
    start_marker: &str,
    end_marker: &str,
) -> Option<(usize, usize)> {
    let start_idx = existing.find(start_marker)?;
    let end_rel_idx = existing[start_idx..].find(end_marker)?;
    let mut end_idx = start_idx + end_rel_idx + end_marker.len();
    if existing[end_idx..].starts_with("\r\n") {
        end_idx += 2;
    } else if existing[end_idx..].starts_with('\n') {
        end_idx += 1;
    }
    Some((start_idx, end_idx))
}

fn upsert_managed_block(existing: &str, managed_block: &str) -> String {
    if let Some((start_idx, end_idx)) =
        managed_block_range(existing, UPKG_BLOCK_START, UPKG_BLOCK_END)
            .or_else(|| managed_block_range(existing, OLD_UPKG_BLOCK_START, OLD_UPKG_BLOCK_END))
    {
        let mut out = String::with_capacity(existing.len() + managed_block.len());
        out.push_str(&existing[..start_idx]);
        out.push_str(managed_block);
        out.push_str(&existing[end_idx..]);
        return out;
    }

    if existing.trim().is_empty() {
        managed_block.to_string()
    } else {
        let mut out = String::with_capacity(existing.len() + managed_block.len() + 1);
        out.push_str(existing);
        if !existing.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(managed_block);
        out
    }
}

fn add_to_path(
    prefix: &Path,
    upkg_dir: &str,
    upkg_bin: &str,
    root: &Path,
    no_modify_path: bool,
) -> Result<(), InitError> {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let home = home()?;

    let config_file = if shell.contains("zsh") {
        let zdotdir = std::env::var("ZDOTDIR").unwrap_or_else(|_| home.clone());
        let zshenv = format!("{}/.zshenv", zdotdir);
        let zshrc = format!("{}/.zshrc", zdotdir);
        let home_zshrc = format!("{}/.zshrc", home);

        if std::path::Path::new(&zshenv).exists() {
            zshenv
        } else if std::path::Path::new(&zshrc).exists() {
            zshrc
        } else {
            home_zshrc
        }
    } else if shell.contains("bash") {
        let bash_profile = format!("{}/.bash_profile", home);
        if std::path::Path::new(&bash_profile).exists() {
            bash_profile
        } else {
            format!("{}/.bashrc", home)
        }
    } else {
        format!("{}/.profile", home)
    };

    let prefix_bin = prefix.join("bin");
    let existing_config = std::fs::read_to_string(&config_file).unwrap_or_default();

    if !no_modify_path {
        let block_body = format!(
            r#"
# upkg
export UPKG_DIR={upkg_dir}
export UPKG_BIN={upkg_bin}
export UPKG_ROOT={root}
export UPKG_PREFIX={prefix}
export PKG_CONFIG_PATH="$UPKG_PREFIX/lib/pkgconfig:${{PKG_CONFIG_PATH:-}}"

# SSL/TLS certificates (only if ca-certificates is installed)
if [ -z "${{CURL_CA_BUNDLE:-}}" ] || [ -z "${{SSL_CERT_FILE:-}}" ]; then
  if [ -f "$UPKG_PREFIX/opt/ca-certificates/share/ca-certificates/cacert.pem" ]; then
    [ -z "${{CURL_CA_BUNDLE:-}}" ] && export CURL_CA_BUNDLE="$UPKG_PREFIX/opt/ca-certificates/share/ca-certificates/cacert.pem"
    [ -z "${{SSL_CERT_FILE:-}}" ] && export SSL_CERT_FILE="$UPKG_PREFIX/opt/ca-certificates/share/ca-certificates/cacert.pem"
  elif [ -f "$UPKG_PREFIX/etc/ca-certificates/cacert.pem" ]; then
    [ -z "${{CURL_CA_BUNDLE:-}}" ] && export CURL_CA_BUNDLE="$UPKG_PREFIX/etc/ca-certificates/cacert.pem"
    [ -z "${{SSL_CERT_FILE:-}}" ] && export SSL_CERT_FILE="$UPKG_PREFIX/etc/ca-certificates/cacert.pem"
  elif [ -f "$UPKG_PREFIX/etc/openssl/cert.pem" ]; then
    [ -z "${{CURL_CA_BUNDLE:-}}" ] && export CURL_CA_BUNDLE="$UPKG_PREFIX/etc/openssl/cert.pem"
    [ -z "${{SSL_CERT_FILE:-}}" ] && export SSL_CERT_FILE="$UPKG_PREFIX/etc/openssl/cert.pem"
  elif [ -f "$UPKG_PREFIX/share/ca-certificates/cacert.pem" ]; then
    [ -z "${{CURL_CA_BUNDLE:-}}" ] && export CURL_CA_BUNDLE="$UPKG_PREFIX/share/ca-certificates/cacert.pem"
    [ -z "${{SSL_CERT_FILE:-}}" ] && export SSL_CERT_FILE="$UPKG_PREFIX/share/ca-certificates/cacert.pem"
  fi
fi

if [ -z "${{SSL_CERT_DIR:-}}" ]; then
  if [ -d "$UPKG_PREFIX/etc/ca-certificates" ]; then
    export SSL_CERT_DIR="$UPKG_PREFIX/etc/ca-certificates"
  elif [ -d "$UPKG_PREFIX/etc/openssl/certs" ]; then
    export SSL_CERT_DIR="$UPKG_PREFIX/etc/openssl/certs"
  elif [ -d "$UPKG_PREFIX/share/ca-certificates" ]; then
    export SSL_CERT_DIR="$UPKG_PREFIX/share/ca-certificates"
  fi
fi

# Helper function to safely append to PATH
_upkg_path_append() {{
    local argpath="$1"
    case ":${{PATH}}:" in
        *:"$argpath":*) ;;
        *) export PATH="$argpath:$PATH" ;;
    esac;
}}

_upkg_path_append "$UPKG_BIN"
_upkg_path_append "$UPKG_PREFIX/bin"
"#,
            upkg_dir = upkg_dir,
            upkg_bin = upkg_bin,
            root = root.display(),
            prefix = prefix.display()
        );
        let managed_block = format!("{UPKG_BLOCK_START}{block_body}\n{UPKG_BLOCK_END}\n");
        let updated_config = upsert_managed_block(&existing_config, &managed_block);

        if let Some(parent) = std::path::Path::new(&config_file).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                InitError::Message(format!(
                    "Failed to create shell config directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        let write_result = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&config_file)
            .and_then(|mut f| f.write_all(updated_config.as_bytes()));

        if let Err(e) = write_result {
            println!(
                "{} Could not write to {} due to error: {}",
                style("Warning:").yellow().bold(),
                config_file,
                e
            );
            println!(
                "{} Please add the following to {}:",
                style("Info:").cyan().bold(),
                config_file
            );
            println!("{}", managed_block);
        } else {
            println!(
                "    {} Updated upkg configuration in {}",
                style("✓").green(),
                config_file
            );
            println!(
                "    {} Added {} and {} to PATH",
                style("✓").green(),
                upkg_bin,
                prefix_bin.display()
            );
        }
    } else if no_modify_path {
        println!(
            "    {} Skipped shell configuration (--no-modify-path)",
            style("→").cyan()
        );
        println!(
            "    {} To use upkg, add {} and {} to your PATH",
            style("→").cyan(),
            upkg_bin,
            prefix_bin.display()
        );
    }

    Ok(())
}

pub fn ensure_init(root: &Path, prefix: &Path, auto_init: bool) -> Result<(), crate::types::Error> {
    if !needs_init(root, prefix) {
        return Ok(());
    }

    let is_interactive = std::io::IsTerminal::is_terminal(&std::io::stdin())
        && std::io::IsTerminal::is_terminal(&std::io::stdout());

    if is_interactive && !auto_init {
        println!(
            "{} upkg needs to be initialized first.",
            style("Note:").yellow().bold()
        );
        println!("    This will create directories at:");
        println!("      • {}", root.display());
        println!("      • {}", prefix.display());
        println!();

        print!("Initialize now? [Y/n] ");
        std::io::stdout().flush().unwrap();

        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
        let input = input.trim();

        if !input.is_empty()
            && !input.eq_ignore_ascii_case("y")
            && !input.eq_ignore_ascii_case("yes")
        {
            return Err(crate::types::Error::StoreCorruption {
                message: "Initialization required.".to_string(),
            });
        }
    }
    if !is_interactive && !auto_init {
        return Err(crate::types::Error::StoreCorruption {
            message: "Initialization required.".to_string(),
        });
    }

    run_init(root, prefix, auto_init).map_err(|e| match e {
        InitError::Message(msg) => crate::types::Error::StoreCorruption { message: msg },
    })
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
