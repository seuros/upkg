//! Real-bottle regression: run after `upkg install postgresql@18`.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(bin: &Path, args: &[&str]) -> Output {
    let output = Command::new(bin)
        .args(args)
        .output()
        .expect("run PostgreSQL binary");
    assert!(
        output.status.success(),
        "{} {:?}\n{}\n{}",
        bin.display(),
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

struct Server {
    bin: PathBuf,
    data: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = Command::new(self.bin.join("pg_ctl"))
            .args([
                "-D",
                self.data.to_str().unwrap(),
                "-m",
                "immediate",
                "-w",
                "stop",
            ])
            .output();
    }
}

#[test]
#[ignore = "requires a real PostgreSQL 18 bottle installed by upkg"]
fn real_postgresql_bottle_can_initialize_start_and_load_extensions() {
    let prefix = std::env::var_os("UPKG_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(target_arch = "aarch64") {
                "/opt/homebrew"
            } else {
                "/usr/local"
            })
        });
    let bin = prefix.join("opt/postgresql@18/bin");
    // macOS limits Unix socket path lengths; don't use the long default TMPDIR.
    let home = tempfile::Builder::new()
        .prefix("upkg-pg-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let data = home.path().join("data");
    let data_str = data.to_str().unwrap();
    run(
        &bin.join("initdb"),
        &[
            "-D",
            data_str,
            "-U",
            "chaos_test",
            "--auth=trust",
            "--encoding=UTF8",
            "--locale=C",
        ],
    );
    let _server = Server {
        bin: bin.clone(),
        data: data.clone(),
    };
    let log = home.path().join("postgres.log");
    let options = format!("-h '' -k {}", home.path().display());
    run(
        &bin.join("pg_ctl"),
        &[
            "-D",
            data_str,
            "-l",
            log.to_str().unwrap(),
            "-o",
            &options,
            "-w",
            "start",
        ],
    );
    let output = run(
        &bin.join("psql"),
        &[
            "-X",
            "-h",
            home.path().to_str().unwrap(),
            "-U",
            "chaos_test",
            "-d",
            "postgres",
            "-v",
            "ON_ERROR_STOP=1",
            "-At",
            "-c",
            "CREATE EXTENSION hstore; SET timezone_abbreviations = 'Default'; SELECT 'key=>value'::hstore -> 'key';",
        ],
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line == "value")
    );
}
