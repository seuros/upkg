use super::*;
use rstest::rstest;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{LazyLock, Mutex, MutexGuard};
use tempfile::TempDir;

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

const UPKG_DIR: &str = "/home/user/.upkg";
const UPKG_BIN: &str = "/home/user/.upkg/bin";

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap()
}

fn set_mode(path: &Path, mode: u32) {
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(mode);
    fs::set_permissions(path, perms).unwrap();
}

/// `(root, prefix)` paths under `tmp`; nothing is created on disk.
fn layout(tmp: &TempDir) -> (PathBuf, PathBuf) {
    (tmp.path().join("root"), tmp.path().join("prefix"))
}

fn create_managed_dirs(root: &Path, prefix: &Path) {
    for dir in managed_dirs(root, prefix) {
        fs::create_dir_all(dir).unwrap();
    }
}

/// Temp `$HOME` with existing root/prefix dirs and `HOME`/`SHELL` exported.
/// Holds the env lock for its whole lifetime.
struct ShellHome {
    tmp: TempDir,
    root: PathBuf,
    prefix: PathBuf,
    _env_lock: MutexGuard<'static, ()>,
}

impl ShellHome {
    #[allow(unsafe_code)]
    fn new(shell: &str) -> Self {
        let env_lock = env_lock();
        let tmp = TempDir::new().unwrap();
        let (root, prefix) = layout(&tmp);
        fs::create_dir(&prefix).unwrap();
        fs::create_dir(&root).unwrap();

        // SAFETY: tests that touch the process environment serialize on ENV_LOCK.
        unsafe {
            std::env::set_var("HOME", tmp.path().to_str().unwrap());
            std::env::set_var("SHELL", shell);
        }

        Self {
            tmp,
            root,
            prefix,
            _env_lock: env_lock,
        }
    }

    fn home(&self) -> &Path {
        self.tmp.path()
    }

    fn add_to_path(&self, no_modify_path: bool) {
        add_to_path(&self.prefix, UPKG_DIR, UPKG_BIN, &self.root, no_modify_path).unwrap();
    }
}

/// Runs `add_to_path` for bash and returns the fixture plus the written `.bashrc`.
fn bashrc_after_add_to_path() -> (ShellHome, String) {
    let home = ShellHome::new("/bin/bash");
    home.add_to_path(false);
    let content = fs::read_to_string(home.home().join(".bashrc")).unwrap();
    (home, content)
}

#[test]
fn needs_init_when_directories_missing() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("nonexistent_root");
    let prefix = tmp.path().join("nonexistent_prefix");

    assert!(needs_init(&root, &prefix));
}

#[test]
fn needs_init_when_not_writable() {
    let tmp = TempDir::new().unwrap();
    let (root, prefix) = layout(&tmp);

    fs::create_dir(&root).unwrap();
    fs::create_dir(&prefix).unwrap();

    set_mode(&root, 0o555);
    let result = needs_init(&root, &prefix);
    set_mode(&root, 0o755);

    assert!(result);
}

#[test]
fn no_init_needed_when_writable() {
    let tmp = TempDir::new().unwrap();
    let (root, prefix) = layout(&tmp);

    create_managed_dirs(&root, &prefix);

    assert!(!needs_init(&root, &prefix));
}

#[test]
fn needs_init_when_db_dir_missing() {
    let tmp = TempDir::new().unwrap();
    let (root, prefix) = layout(&tmp);

    create_managed_dirs(&root, &prefix);
    fs::remove_dir(root.join("db")).unwrap();

    assert!(needs_init(&root, &prefix));
}

#[test]
fn is_writable_returns_true_for_writable_dir() {
    let tmp = TempDir::new().unwrap();
    assert!(is_writable(tmp.path()));
}

#[test]
fn is_writable_returns_false_for_nonexistent_path() {
    let tmp = TempDir::new().unwrap();
    let nonexistent = tmp.path().join("does_not_exist");
    assert!(!is_writable(&nonexistent));
}

#[test]
fn is_writable_returns_false_for_readonly_dir() {
    let tmp = TempDir::new().unwrap();
    let readonly = tmp.path().join("readonly");
    fs::create_dir(&readonly).unwrap();

    set_mode(&readonly, 0o555);
    assert!(!is_writable(&readonly));
    set_mode(&readonly, 0o755);
}

#[test]
fn shell_quote_round_trips_single_quotes() {
    let quoted = shell_quote("owner's prefix").unwrap();
    let words = shlex::split(&format!("cmd {quoted}")).unwrap();
    assert_eq!(words, vec!["cmd", "owner's prefix"]);
}

#[test]
fn add_to_path_writes_core_env_vars_with_guarded_ca_setup() {
    let (home, content) = bashrc_after_add_to_path();

    assert!(content.contains(UPKG_BLOCK_START));
    assert!(content.contains(UPKG_BLOCK_END));
    assert!(content.contains("export UPKG_DIR=/home/user/.upkg"));
    assert!(content.contains("export UPKG_BIN=/home/user/.upkg/bin"));
    assert!(content.contains(&format!("export UPKG_ROOT={}", home.root.display())));
    assert!(content.contains(&format!("export UPKG_PREFIX={}", home.prefix.display())));
    assert!(content.contains("export PKG_CONFIG_PATH="));
    assert!(content.contains("/lib/pkgconfig"));
    assert!(
        content
            .contains("if [ -z \"${CURL_CA_BUNDLE:-}\" ] || [ -z \"${SSL_CERT_FILE:-}\" ]; then")
    );
    assert!(content.contains("if [ -z \"${SSL_CERT_DIR:-}\" ]; then"));
    assert!(content.contains("CURL_CA_BUNDLE"));
    assert!(content.contains("SSL_CERT_FILE"));
    assert!(content.contains("SSL_CERT_DIR"));
    assert!(content.contains("$UPKG_PREFIX/etc/openssl/cert.pem"));
    assert!(content.contains("$UPKG_PREFIX/etc/openssl/certs"));
}

#[test]
fn add_to_path_includes_path_append_function() {
    let (_home, content) = bashrc_after_add_to_path();

    assert!(content.contains("_upkg_path_append()"));
    assert!(content.contains("case \":${PATH}:"));
    assert!(content.contains("_upkg_path_append"));
}

#[test]
fn add_to_path_adds_both_paths() {
    let (_home, content) = bashrc_after_add_to_path();

    assert!(content.contains("_upkg_path_append \"$UPKG_BIN\""));
    assert!(content.contains("_upkg_path_append \"$UPKG_PREFIX/bin\""));
}

#[test]
fn add_to_path_no_modify_shell_skips_write() {
    let home = ShellHome::new("/bin/bash");

    home.add_to_path(true);

    assert!(!home.home().join(".bashrc").exists());
}

#[test]
fn add_to_path_no_duplicate_config() {
    let home = ShellHome::new("/bin/bash");
    let shell_config = home.home().join(".bashrc");

    fs::write(
        &shell_config,
        format!(
            "export KEEP_ME=true\n{UPKG_BLOCK_START}\n# upkg\nexport UPKG_DIR=/old\n{UPKG_BLOCK_END}\n"
        ),
    )
    .unwrap();

    home.add_to_path(false);

    let content = fs::read_to_string(&shell_config).unwrap();
    assert!(content.contains("export KEEP_ME=true"));
    assert!(content.contains(&format!("export UPKG_DIR={UPKG_DIR}")));
    assert!(!content.contains("export UPKG_DIR=/old"));
    assert_eq!(content.matches(UPKG_BLOCK_START).count(), 1);
    assert_eq!(content.matches(UPKG_BLOCK_END).count(), 1);
}

#[rstest]
#[case("/bin/zsh", None, None, ".zshrc", "zsh defaults to .zshrc")]
#[case(
    "/bin/zsh",
    Some(".zshenv"),
    None,
    ".zshenv",
    "zsh prefers .zshenv when exists"
)]
#[case(
    "/bin/bash",
    Some(".bash_profile"),
    None,
    ".bash_profile",
    "bash prefers .bash_profile when exists"
)]
#[case("/bin/bash", None, None, ".bashrc", "bash defaults to .bashrc")]
#[case("/bin/sh", None, None, ".profile", "sh uses .profile")]
#[case(
    "/bin/zsh",
    Some("zsh_config/.zshrc"),
    Some("zsh_config"),
    "zsh_config/.zshrc",
    "zsh uses ZDOTDIR when file exists"
)]
#[case(
    "/bin/zsh",
    None,
    Some("zsh_config"),
    ".zshrc",
    "zsh falls back to home .zshrc when ZDOTDIR files missing"
)]
fn shell_config_file_selection(
    #[case] shell: &str,
    #[case] existing_file: Option<&str>,
    #[case] zdotdir: Option<&str>,
    #[case] expected_file: &str,
    #[case] _description: &str,
) {
    let fixture = ShellHome::new(shell);
    let home = fixture.home();

    // Create existing file if specified
    if let Some(file) = existing_file {
        let path = home.join(file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, "# existing\n").unwrap();
    }

    // Setup ZDOTDIR if specified
    if let Some(zdir) = zdotdir {
        let zdotdir_path = home.join(zdir);
        fs::create_dir_all(&zdotdir_path).unwrap();
        #[allow(unsafe_code)]
        // SAFETY: serialized by the env lock `ShellHome` holds.
        unsafe {
            std::env::set_var("ZDOTDIR", zdotdir_path.to_str().unwrap());
        }
    } else {
        #[allow(unsafe_code)]
        // SAFETY: serialized by the env lock `ShellHome` holds.
        unsafe {
            std::env::remove_var("ZDOTDIR");
        }
    }

    fixture.add_to_path(false);

    let config_file = home.join(expected_file);
    assert!(
        config_file.exists(),
        "Expected {} to exist for shell {}",
        expected_file,
        shell
    );
    let content = fs::read_to_string(&config_file).unwrap();
    assert!(content.contains("# upkg"));
}

#[test]
fn upsert_managed_block_replacement_consumes_trailing_newline() {
    let managed_block =
        format!("{UPKG_BLOCK_START}\n# upkg\nexport UPKG_DIR=/new\n{UPKG_BLOCK_END}\n");
    let existing = format!(
        "prefix\n{UPKG_BLOCK_START}\n# upkg\nexport UPKG_DIR=/old\n{UPKG_BLOCK_END}\npostfix\n"
    );

    let first = upsert_managed_block(&existing, &managed_block);
    let second = upsert_managed_block(&first, &managed_block);

    assert_eq!(first, second);
    assert!(first.contains("# <<< upkg <<<\npostfix\n"));
    assert!(!first.contains("# <<< upkg <<<\n\npostfix\n"));
}

#[test]
fn upsert_managed_block_replaces_legacy_native_block() {
    let managed_block =
        format!("{UPKG_BLOCK_START}\n# upkg\nexport UPKG_DIR=/new\n{UPKG_BLOCK_END}\n");
    let existing = format!(
        "prefix\n{OLD_UPKG_BLOCK_START}\n# upkg-native\nexport UPKG_DIR=/old\n{OLD_UPKG_BLOCK_END}\npostfix\n"
    );

    let updated = upsert_managed_block(&existing, &managed_block);

    assert!(updated.contains("export UPKG_DIR=/new"));
    assert!(!updated.contains("export UPKG_DIR=/old"));
    assert!(!updated.contains(OLD_UPKG_BLOCK_START));
    assert!(updated.contains("# <<< upkg <<<\npostfix\n"));
}
