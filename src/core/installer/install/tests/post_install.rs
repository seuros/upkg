use super::*;
use crate::core::storage::receipt::read_receipt;

/// Serves a PostgreSQL-shaped bottle named `name` and returns a fresh context.
async fn postgres_context(name: &str) -> (MockServer, TempDir, TestInstallContext) {
    let server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();
    mount_postgres(&server, name).await;
    let ctx = server_context(&server, &tmp);
    (server, tmp, ctx)
}

async fn mount_postgres(server: &MockServer, name: &str) {
    let bottle = tar_gz(
        [
            ("share/postgresql/postgres.bki", "bootstrap"),
            ("lib/postgresql/plpgsql.so", "library"),
            ("include/postgresql/server.h", "header"),
            ("bin/initdb", "#!/bin/sh\nexit 1\n"),
        ]
        .map(|(file, content)| (format!("{name}/18.6/{file}"), content)),
    );
    let formula = serde_json::json!({
        "name": name,
        "versions": { "stable": "18.6" },
        "dependencies": [],
        "keg_only": name.contains('@'),
        "bottle": { "stable": { "files": {
            get_test_bottle_tag(): { "url": format!("{}/postgres.tar.gz", server.uri()),
                                    "sha256": sha256_hex(&bottle) }
        }}},
        "post_install_steps": [
            {"type":"link_dir", "source":{"base":"prefix","path":"share/postgresql"},
             "target":{"base":"homebrew_prefix","path":"share/{{formula_name}}"}},
            {"type":"link_dir", "source":{"base":"prefix","path":"lib/postgresql"},
             "target":{"base":"homebrew_prefix","path":"lib/{{formula_name}}"}},
            {"type":"link_dir", "source":{"base":"prefix","path":"include/postgresql"},
             "target":{"base":"homebrew_prefix","path":"include/{{formula_name}}"}},
            {"type":"link_children", "source":{"base":"prefix","path":"bin"},
             "target":{"base":"homebrew_prefix","path":"bin"}, "suffix":"-{{version.major}}"}
        ]
    });
    mount_json(server, &format!("/{name}.json"), formula).await;
    mount_bytes(server, "/postgres.tar.gz", bottle).await;
}

#[tokio::test]
async fn keg_only_install_links_runtime_paths_even_with_no_link() {
    for link in [true, false] {
        let (_server, _tmp, mut ctx) = postgres_context("postgresql@18").await;
        ctx.installer
            .install(&["postgresql@18".into()], link)
            .await
            .unwrap();
        assert!(ctx.prefix.join("share/postgresql@18/postgres.bki").exists());
        assert!(ctx.prefix.join("lib/postgresql@18/plpgsql.so").exists());
        assert!(ctx.prefix.join("include/postgresql@18/server.h").exists());
        assert!(ctx.prefix.join("bin/initdb-18").exists());
        assert!(!ctx.prefix.join("bin/initdb").exists());
        assert!(
            !read_receipt(&ctx.root.join("Cellar/postgresql@18/18.6"))
                .unwrap()
                .post_install_steps
                .is_empty()
        );
        ctx.installer.uninstall("postgresql@18").unwrap();
        assert!(
            !ctx.prefix
                .join("share/postgresql@18/postgres.bki")
                .is_symlink()
        );
        assert!(!ctx.prefix.join("bin/initdb-18").is_symlink());
    }
}

#[tokio::test]
async fn store_reinstall_restores_runtime_links_without_fetching_formula_metadata() {
    let (server, _tmp, mut ctx) = postgres_context("postgresql@18").await;
    ctx.installer
        .install(&["postgresql@18".into()], false)
        .await
        .unwrap();
    // A runtime link can be missing without changing the keg or cached bottle.
    fs::remove_file(ctx.prefix.join("share/postgresql@18/postgres.bki")).unwrap();
    server.reset().await;
    let plan = ctx
        .installer
        .plan_reinstall(&["postgresql@18".into()], false)
        .await
        .unwrap();
    assert_eq!(plan.from_store.len(), 1);
    ctx.installer
        .reinstall_with_progress(plan, false, None)
        .await
        .unwrap();
    assert!(ctx.prefix.join("share/postgresql@18/postgres.bki").exists());
    assert!(ctx.prefix.join("bin/initdb-18").exists());
    assert!(!ctx.prefix.join("bin/initdb").exists());
}

#[tokio::test]
async fn conflicting_post_install_paths_do_not_mark_the_package_installed() {
    let (_server, _tmp, mut ctx) = postgres_context("postgresql@18").await;
    let conflict = ctx.prefix.join("lib/postgresql@18/plpgsql.so");
    fs::create_dir_all(conflict.parent().unwrap()).unwrap();
    fs::write(&conflict, "user library").unwrap();
    assert!(
        ctx.installer
            .install(&["postgresql@18".into()], true)
            .await
            .is_err()
    );
    assert!(!ctx.installer.is_installed("postgresql@18"));
    assert!(
        !ctx.prefix
            .join("share/postgresql@18/postgres.bki")
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(conflict).unwrap(), "user library");
}

#[tokio::test]
async fn source_install_also_applies_and_records_declarative_runtime_links() {
    let server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();
    let ruby = tmp.path().join("sourcepkg.rb");
    fs::write(
        &ruby,
        "class Sourcepkg < Formula\n  def install\n    share.install \"runtime\"\n  end\nend\n",
    )
    .unwrap();
    let source = tar_gz([("source/runtime/postgres.bki", "bootstrap")]);
    let formula = serde_json::json!({
        "name":"sourcepkg", "versions":{"stable":"1.0"}, "dependencies":[],
        "bottle":{"stable":{"files":{}}},
        "urls":{"stable":{"url":format!("{}/source.tar.gz", server.uri()),
                          "checksum":sha256_hex(&source)}},
        "ruby_source_path":ruby.to_str().unwrap(),
        "post_install_steps":[
            {"type":"link_dir", "source":{"base":"prefix","path":"share/runtime"},
             "target":{"base":"homebrew_prefix","path":"share/{{formula_name}}"}}
        ]
    });
    mount_json(&server, "/sourcepkg.json", formula).await;
    mount_bytes(&server, "/source.tar.gz", source).await;
    let mut ctx = server_context(&server, &tmp);
    // Source BuildPlans use the prefix's real Cellar layout.
    ctx.installer.cellar = Cellar::new_at(ctx.prefix.join("Cellar")).unwrap();
    ctx.installer
        .install(&["sourcepkg".into()], false)
        .await
        .unwrap();
    assert!(ctx.prefix.join("share/sourcepkg/postgres.bki").exists());
    let receipt = read_receipt(&ctx.prefix.join("Cellar/sourcepkg/1.0")).unwrap();
    assert_eq!(receipt.post_install_steps.len(), 1);
    ctx.installer.uninstall("sourcepkg").unwrap();
    assert!(!ctx.prefix.join("share/sourcepkg/postgres.bki").is_symlink());
}

#[tokio::test]
async fn ordinary_global_link_conflict_does_not_disable_required_runtime_paths() {
    let (_server, _tmp, mut ctx) = postgres_context("postgresql").await;
    fs::write(ctx.prefix.join("bin/initdb"), "user binary").unwrap();
    assert!(
        ctx.installer
            .install(&["postgresql".into()], true)
            .await
            .is_err()
    );
    // The keg was installed successfully; only ordinary global linking failed.
    assert!(ctx.installer.is_installed("postgresql"));
    assert!(ctx.prefix.join("opt/postgresql").exists());
    assert!(ctx.prefix.join("share/postgresql/postgres.bki").exists());
    assert!(ctx.prefix.join("lib/postgresql/plpgsql.so").exists());
    assert!(ctx.prefix.join("bin/initdb-18").exists());
    assert_eq!(
        fs::read_to_string(ctx.prefix.join("bin/initdb")).unwrap(),
        "user binary"
    );
}
