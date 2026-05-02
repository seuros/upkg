use super::*;

#[test]
fn stage_cask_apps_moves_app_and_leaves_caskroom_symlink() {
    let tmp = TempDir::new().unwrap();
    let source_root = tmp.path().join("mounted");
    let source_app = source_root.join("Ghostty.app");
    let staged_path = tmp.path().join("homebrew/Caskroom/ghostty/1.3.1");
    let prefix = tmp.path().join("homebrew");

    fs::create_dir_all(source_app.join("Contents")).unwrap();
    fs::write(source_app.join("Contents/Info.plist"), "ghostty").unwrap();
    fs::create_dir_all(&staged_path).unwrap();

    let cask = crate::core::installer::cask::ResolvedCask {
        install_name: "cask:ghostty".to_string(),
        token: "ghostty".to_string(),
        version: "1.3.1".to_string(),
        url: "https://example.com/Ghostty.dmg".to_string(),
        sha256: "abc".to_string(),
        binaries: Vec::new(),
        apps: vec![crate::core::installer::cask::CaskApp {
            source: "Ghostty.app".to_string(),
            target: "Ghostty.app".to_string(),
        }],
        linked_artifacts: Vec::new(),
    };

    stage_cask_apps(&source_root, &staged_path, &prefix, &cask).unwrap();

    let target_app = prefix.join("Applications/Ghostty.app");
    assert!(target_app.join("Contents/Info.plist").exists());
    assert!(staged_path.join("Ghostty.app").is_symlink());
    assert_eq!(
        fs::read_link(staged_path.join("Ghostty.app")).unwrap(),
        target_app
    );
}

#[test]
fn stage_cask_linked_artifacts_links_appdir_sources_into_prefix() {
    let tmp = TempDir::new().unwrap();
    let source_root = tmp.path().join("mounted");
    let prefix = tmp.path().join("homebrew");
    let app_resources = prefix.join("Applications/Ghostty.app/Contents/Resources");

    fs::create_dir_all(app_resources.join("man/man1")).unwrap();
    fs::create_dir_all(app_resources.join("bash-completion/completions")).unwrap();
    fs::write(app_resources.join("man/man1/ghostty.1"), "man").unwrap();
    fs::write(
        app_resources.join("bash-completion/completions/ghostty.bash"),
        "complete",
    )
    .unwrap();

    let cask = crate::core::installer::cask::ResolvedCask {
            install_name: "cask:ghostty".to_string(),
            token: "ghostty".to_string(),
            version: "1.3.1".to_string(),
            url: "https://example.com/Ghostty.dmg".to_string(),
            sha256: "abc".to_string(),
            binaries: Vec::new(),
            apps: Vec::new(),
            linked_artifacts: vec![
                crate::core::installer::cask::CaskLinkedArtifact {
                    kind: crate::core::installer::cask::CaskLinkedArtifactKind::Manpage,
                    source:
                        "$APPDIR/Ghostty.app/Contents/Resources/man/man1/ghostty.1".to_string(),
                    target: "share/man/man1/ghostty.1".to_string(),
                },
                crate::core::installer::cask::CaskLinkedArtifact {
                    kind: crate::core::installer::cask::CaskLinkedArtifactKind::BashCompletion,
                    source: "$APPDIR/Ghostty.app/Contents/Resources/bash-completion/completions/ghostty.bash".to_string(),
                    target: "etc/bash_completion.d/ghostty".to_string(),
                },
            ],
        };

    stage_cask_linked_artifacts(&source_root, &prefix, &cask).unwrap();

    assert_eq!(
        fs::read_link(prefix.join("share/man/man1/ghostty.1")).unwrap(),
        app_resources.join("man/man1/ghostty.1")
    );
    assert_eq!(
        fs::read_link(prefix.join("etc/bash_completion.d/ghostty")).unwrap(),
        app_resources.join("bash-completion/completions/ghostty.bash")
    );
}

#[test]
fn uninstall_cask_removes_app_and_caskroom() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("upkg");
    let prefix = tmp.path().join("homebrew");
    let caskroom_app = prefix.join("Caskroom/ghostty/1.3.1/Ghostty.app");
    let target_app = prefix.join("Applications/Ghostty.app");
    let manpage_source = target_app.join("Contents/Resources/man/man1/ghostty.1");
    let manpage_link = prefix.join("share/man/man1/ghostty.1");
    let metadata_cask =
        prefix.join("Caskroom/ghostty/.metadata/1.3.1/20260502093557.000/Casks/ghostty.json");

    fs::create_dir_all(target_app.join("Contents")).unwrap();
    fs::create_dir_all(manpage_source.parent().unwrap()).unwrap();
    fs::write(&manpage_source, "man").unwrap();
    fs::create_dir_all(manpage_link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&manpage_source, &manpage_link).unwrap();
    fs::create_dir_all(caskroom_app.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target_app, &caskroom_app).unwrap();
    fs::create_dir_all(metadata_cask.parent().unwrap()).unwrap();
    write_json_pretty(
        &metadata_cask,
        &serde_json::json!({
            "token": "ghostty",
            "version": "1.3.1",
            "url": "https://example.com/Ghostty.dmg",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [
                { "app": ["Ghostty.app"] },
                { "manpage": ["$APPDIR/Ghostty.app/Contents/Resources/man/man1/ghostty.1"] }
            ]
        }),
    )
    .unwrap();

    let api_client = ApiClient::new();
    let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
    let store = Store::new(&root).unwrap();
    let cellar = Cellar::new(&root).unwrap();
    let linker = Linker::new(&prefix).unwrap();

    let mut installer = Installer::new(api_client, blob_cache, store, cellar, linker, prefix);

    installer.uninstall("cask:ghostty").unwrap();

    assert!(!target_app.exists());
    assert!(!manpage_link.exists());
    assert!(!tmp.path().join("homebrew/Caskroom/ghostty").exists());
}
