use super::*;
use std::assert_matches;

#[test]
fn resolve_cask_uses_platform_variation_url_and_sha() {
    let mut cask = serde_json::json!({
        "token": "test",
        "version": "1.0.0",
        "url": "https://example.com/darwin.zip",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [{ "binary": [["op"]] }],
        "variations": {}
    });
    let variation_key = current_macos_variation_key();
    cask["variations"][variation_key.as_str()] = serde_json::json!({
        "url": "https://example.com/macos.zip",
        "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    });

    let resolved = resolve_cask("test", &cask).unwrap();
    assert_eq!(resolved.url, "https://example.com/macos.zip");
    assert_eq!(
        resolved.sha256,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
}

#[test]
fn resolve_cask_parses_binary_targets() {
    let cask = serde_json::json!({
        "token": "test",
        "version": "1.0.0",
        "url": "https://example.com/test.zip",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [{
            "binary": [
                ["bin/tool"],
                ["bin/tool2", {"target": "tool-two"}]
            ]
        }]
    });

    let resolved = resolve_cask("test", &cask).unwrap();
    assert_eq!(resolved.binaries.len(), 2);
    assert_eq!(resolved.binaries[0].target, "bin/tool");
    assert_eq!(resolved.binaries[1].target, "bin/tool-two");
}

#[test]
fn resolve_cask_parses_app_artifacts() {
    let cask = serde_json::json!({
        "token": "ghostty",
        "version": "1.0.0",
        "url": "https://example.com/Ghostty.dmg",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [{
            "app": ["Ghostty.app"]
        }]
    });

    let resolved = resolve_cask("ghostty", &cask).unwrap();
    assert!(resolved.binaries.is_empty());
    assert_eq!(resolved.apps.len(), 1);
    assert_eq!(resolved.apps[0].source, "Ghostty.app");
    assert_eq!(resolved.apps[0].target, "Ghostty.app");
}

#[test]
fn resolve_cask_parses_pkg_artifacts_and_uninstall_directives() {
    let cask = serde_json::json!({
        "token": "test-pkg",
        "version": "1.0.0",
        "url": "https://example.com/Test.dmg",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [
            { "uninstall": [{
                "pkgutil": ["com.example.test", "com.example.helper"],
                "delete": ["/Library/Application Support/Test"]
            }] },
            { "pkg": ["Test Installer.pkg"] }
        ]
    });

    let resolved = resolve_cask("test-pkg", &cask).unwrap();

    assert!(resolved.apps.is_empty());
    assert!(resolved.binaries.is_empty());
    assert_eq!(resolved.pkgs.len(), 1);
    assert_eq!(resolved.pkgs[0].source, "Test Installer.pkg");
    assert_eq!(
        resolved.uninstall.pkgutil,
        vec!["com.example.test", "com.example.helper"]
    );
    assert_eq!(
        resolved.uninstall.delete,
        vec!["/Library/Application Support/Test"]
    );
}

#[test]
fn resolve_cask_parses_string_pkgutil_uninstall_directive() {
    let cask = serde_json::json!({
        "token": "test-pkg",
        "version": "1.0.0",
        "url": "https://example.com/Test.pkg",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [
            { "uninstall": [{ "pkgutil": "com.example.*" }] },
            { "pkg": ["Test.pkg", { "allow_untrusted": true }] }
        ]
    });

    let resolved = resolve_cask("test-pkg", &cask).unwrap();

    assert_eq!(resolved.pkgs.len(), 1);
    assert_eq!(resolved.pkgs[0].source, "Test.pkg");
    assert_eq!(resolved.uninstall.pkgutil, vec!["com.example.*"]);
}

#[test]
fn resolve_cask_parses_secondary_artifacts() {
    let cask = serde_json::json!({
        "token": "ghostty",
        "version": "1.0.0",
        "url": "https://example.com/Ghostty.dmg",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [
            { "app": ["Ghostty.app"] },
            { "manpage": ["$APPDIR/Ghostty.app/Contents/Resources/man/man1/ghostty.1"] },
            { "manpage": ["$APPDIR/Ghostty.app/Contents/Resources/man/man5/ghostty.5"] },
            { "bash_completion": ["$APPDIR/Ghostty.app/Contents/Resources/bash-completion/completions/ghostty.bash"] },
            { "fish_completion": ["$APPDIR/Ghostty.app/Contents/Resources/fish/vendor_completions.d/ghostty.fish"] },
            { "zsh_completion": ["$APPDIR/Ghostty.app/Contents/Resources/zsh/site-functions/_ghostty"] }
        ]
    });

    let resolved = resolve_cask("ghostty", &cask).unwrap();
    let targets: Vec<_> = resolved
        .linked_artifacts
        .iter()
        .map(|artifact| artifact.target.as_str())
        .collect();
    assert_eq!(
        targets,
        vec![
            "share/man/man1/ghostty.1",
            "share/man/man5/ghostty.5",
            "etc/bash_completion.d/ghostty",
            "share/fish/vendor_completions.d/ghostty.fish",
            "share/zsh/site-functions/_ghostty"
        ]
    );
}

#[test]
fn resolve_cask_parses_flat_binary_artifact() {
    let cask = serde_json::json!({
        "token": "gimp",
        "version": "3.2.4",
        "url": "https://example.com/gimp.dmg",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [
            { "app": ["GIMP.app"] },
            { "binary": [
                "$HOMEBREW_PREFIX/Caskroom/gimp/3.2.4/gimp.wrapper.sh",
                { "target": "gimp" }
            ]}
        ]
    });

    let resolved = resolve_cask("gimp", &cask).unwrap();
    assert_eq!(resolved.apps.len(), 1);
    assert_eq!(resolved.apps[0].source, "GIMP.app");
    assert_eq!(resolved.binaries.len(), 1);
    assert_eq!(
        resolved.binaries[0].source,
        "$HOMEBREW_PREFIX/Caskroom/gimp/3.2.4/gimp.wrapper.sh"
    );
    assert_eq!(resolved.binaries[0].target, "bin/gimp");
}

#[test]
fn resolve_cask_parses_flat_app_artifact_with_target() {
    let cask = serde_json::json!({
        "token": "test",
        "version": "1.0.0",
        "url": "https://example.com/test.dmg",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [{
            "app": ["MyApp.app", { "target": "Custom.app" }]
        }]
    });

    let resolved = resolve_cask("test", &cask).unwrap();
    assert_eq!(resolved.apps.len(), 1);
    assert_eq!(resolved.apps[0].source, "MyApp.app");
    assert_eq!(resolved.apps[0].target, "Custom.app");
}

#[test]
fn resolve_cask_parses_multiple_binary_entries() {
    let cask = serde_json::json!({
        "token": "test",
        "version": "1.0.0",
        "url": "https://example.com/test.zip",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [{
            "binary": [
                ["bin/tool"],
                ["bin/tool2", {"target": "tool-two"}]
            ]
        }]
    });

    let resolved = resolve_cask("test", &cask).unwrap();
    assert_eq!(resolved.binaries.len(), 2);
    assert_eq!(resolved.binaries[0].source, "bin/tool");
    assert_eq!(resolved.binaries[0].target, "bin/tool");
    assert_eq!(resolved.binaries[1].source, "bin/tool2");
    assert_eq!(resolved.binaries[1].target, "bin/tool-two");
}

#[test]
fn resolve_cask_parses_docker_desktop_artifacts() {
    let cask = serde_json::json!({
        "token": "docker-desktop",
        "old_tokens": ["docker"],
        "version": "4.88.1,237512",
        "url": "https://example.com/Docker.dmg",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [
            {
                "app": ["Docker.app"],
                "target": "/Applications/Docker.app"
            },
            {
                "binary": [
                    "$APPDIR/Docker.app/Contents/Resources/bin/docker",
                    {"target": "/usr/local/bin/docker"}
                ],
                "target": "/usr/local/bin/docker"
            },
            {
                "binary": [
                    "$APPDIR/Docker.app/Contents/Resources/cli-plugins/docker-compose",
                    {"target": "/usr/local/cli-plugins/docker-compose"}
                ],
                "target": "/usr/local/cli-plugins/docker-compose"
            },
            {
                "fish_completion": [
                    "$APPDIR/Docker.app/Contents/Resources/etc/docker.fish-completion"
                ],
                "target": "$HOMEBREW_PREFIX/share/fish/vendor_completions.d/docker.fish"
            },
            {
                "zsh_completion": [
                    "$APPDIR/Docker.app/Contents/Resources/etc/docker.zsh-completion"
                ],
                "target": "$HOMEBREW_PREFIX/share/zsh/site-functions/_docker"
            },
            {
                "postflight_steps": [{
                    "steps": [{
                        "source": {
                            "path": "{{appdir}}/Docker.app/Contents/Resources/bin/kubectl"
                        },
                        "target": {"path": "/usr/local/bin/kubectl"},
                        "uninstall": true,
                        "guards": [{
                            "path": "/usr/local/bin/kubectl",
                            "condition": "unless_exists"
                        }],
                        "type": "symlink"
                    }]
                }]
            }
        ]
    });

    let resolved = resolve_cask("docker", &cask).unwrap();

    assert_eq!(resolved.token, "docker-desktop");
    assert_eq!(resolved.install_name, "cask:docker-desktop");
    assert_eq!(resolved.apps[0].target, "Docker.app");
    assert_eq!(resolved.binaries[0].target, "bin/docker");
    assert_eq!(resolved.binaries[1].target, "cli-plugins/docker-compose");
    assert_eq!(
        resolved.linked_artifacts[0].target,
        "share/fish/vendor_completions.d/docker.fish"
    );
    assert_eq!(
        resolved.linked_artifacts[1].target,
        "share/zsh/site-functions/_docker"
    );
    assert_eq!(resolved.postflight_symlinks.len(), 1);
    assert_eq!(
        resolved.postflight_symlinks[0].source,
        "$APPDIR/Docker.app/Contents/Resources/bin/kubectl"
    );
    assert_eq!(resolved.postflight_symlinks[0].target, "bin/kubectl");
    assert!(resolved.postflight_symlinks[0].skip_if_exists);
    assert!(resolved.postflight_symlinks[0].uninstall);
}

#[test]
fn resolve_cask_rejects_binary_targets_outside_known_prefixes() {
    let cask = serde_json::json!({
        "token": "test",
        "version": "1.0.0",
        "url": "https://example.com/test.zip",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [{
            "binary": ["bin/tool", {"target": "/tmp/tool"}]
        }]
    });

    let err = resolve_cask("test", &cask).unwrap_err();
    assert_matches!(err, Error::InvalidArgument { .. });
}

#[test]
fn resolve_cask_missing_required_field_is_invalid_argument() {
    let cask = serde_json::json!({
        "token": "test",
        "version": "1.0.0",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [{ "binary": [["op"]] }]
    });

    let err = resolve_cask("test", &cask).unwrap_err();
    assert_matches!(err, Error::InvalidArgument { .. });
}

#[test]
fn resolve_cask_missing_artifacts_array_is_invalid_argument() {
    let cask = serde_json::json!({
        "token": "test",
        "version": "1.0.0",
        "url": "https://example.com/test.zip",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    });

    let err = resolve_cask("test", &cask).unwrap_err();
    assert_matches!(err, Error::InvalidArgument { .. });
}
