# upkg

Ever tried to document how to install your own program?

You end up writing 47 pages just to say "install the dev headers for libfoo" — because on Debian it's `libfoo-dev`, on Fedora it's `libfoo-devel`, on Arch it's `libfoo`, on Alpine it's `libfoo-dev` (different one), on FreeBSD it's `foo`, and on macOS it's *"fook yu"* — because the dev didn't pay $199 to sign the package.

**upkg lets you speak civilized. It speaks the dialect of 87 other package managers for you.**

```bash
upkg install git curl ripgrep
```

That's it. Same command on Debian, Fedora, Arch, openSUSE, OpenWrt, Termux, FreeBSD, and macOS.

## What it actually does

- One command surface: `upkg install`, `upkg uninstall`, `upkg upgrade`.
- Routes to the native package manager underneath.
- Keeps the names people already know — no new catalog, no parallel universe.
- Stays out of your way for anything advanced. Use `apt`, `dnf`, `pacman`, `pkg` directly when you need to.

## What it is not

- Not a new global package ecosystem.
- Not a replacement for native tooling when you need flags, repos, or version pinning.
- Not a daemon, not a sync service, not a wrapper that "knows better."

## Backends

| Platform | Backend |
|---|---|
| Debian/Ubuntu | `apt` |
| Fedora/RHEL | `dnf` / `yum` |
| Arch | `pacman` |
| openSUSE | `zypper` |
| OpenWrt | `opkg` |
| Android (Termux) | `pkg` (fallback `apt`) |
| FreeBSD | `pkg` |
| macOS | built-in engine (Homebrew-compatible names + prefixes) |

## macOS

No runtime switch, no shelling out to `brew`. The engine is built in and keeps the prefixes people expect:

- Apple Silicon: `/opt/homebrew`
- Intel: `/usr/local`

Same formula and cask names. Different engine.

For app casks, use `--app`:

```bash
upkg install --app ghostty
upkg uninstall --app ghostty
```

App installs are written in the Homebrew-compatible cask layout, so `brew
info --cask <name>` can detect them. Supported cask artifacts currently include
apps, manpages, and bash/fish/zsh completions.

## Usage

```bash
upkg install git curl
upkg install --app ghostty
upkg install --dry-run ripgrep
upkg uninstall htop
upkg uninstall --app ghostty
upkg upgrade
upkg --self-upgrade
upkg help
```

## Build

```bash
cargo build -p upkg
just qq
```
