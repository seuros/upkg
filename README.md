# upkg

Ever tried to document how to install your own program?

You end up writing 47 pages just to say "install the dev headers for libfoo", because on Debian it's `libfoo-dev`, on Fedora it's `libfoo-devel`, on Arch it's `libfoo`, on Alpine it's `libfoo-dev` (different one), on FreeBSD it's `foo`, and on macOS it's *"unidentified developer"*, because the dev didn't pay Apple's yearly tax to bless the binary.

**upkg lets you speak civilized. It speaks the dialect of 87 other package managers for you.**

```bash
upkg install git curl ripgrep
```

That's it. Same command on Debian, Fedora, Arch, openSUSE, OpenWrt, Termux, FreeBSD, DragonFly BSD, and macOS.

## What it actually does

- One command surface: `upkg install`, `upkg uninstall`, `upkg upgrade`.
- Routes to the platform package manager where that is the right boundary, with a built-in Homebrew-compatible engine on macOS.
- Keeps the names people already know: no new catalog, no parallel universe.
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
| FreeBSD (+ GhostBSD, HardenedBSD) | `pkg` |
| DragonFly BSD | `pkg` (DPorts) |
| Any of the above with Ravenports | `rvn` (takes precedence when installed) |
| macOS | built-in engine (Homebrew-compatible names + prefixes) |

## macOS

No runtime switch, no shelling out to `brew`. The engine is built in and keeps the prefixes people expect:

- Apple Silicon: `/opt/homebrew`
- Intel: `/usr/local`

Same formula and cask names. Different engine.

Apps are detected automatically when a name exists as a cask but not as a
formula. Use `--app` when you want to force cask resolution:

```bash
upkg install ghostty
upkg install --app ghostty
upkg uninstall --app ghostty
```

App installs are written in the Homebrew-compatible cask layout, so `brew
info --cask <name>` can detect them. Supported cask artifacts currently include
apps, manpages, and bash/fish/zsh completions.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/seuros/upkg/master/install.sh | sh
```

Or specify a custom install directory:

```bash
curl -fsSL https://raw.githubusercontent.com/seuros/upkg/master/install.sh | UPKG_INSTALL_DIR="$HOME/.local/bin" sh
```

If the install directory is not already in `PATH`, the installer adds it to
your shell profile (`.zshrc`, `.bashrc`, fish config, or `.profile`). Restart
your shell after install. Set `UPKG_NO_MODIFY_PATH=1` to skip this.

## Usage

```bash
upkg install git curl
upkg install ghostty
upkg install --app ghostty
upkg install --dry-run ripgrep
upkg uninstall htop
upkg uninstall --app ghostty
upkg upgrade
upkg list
upkg search ripgrep
upkg search --app ghostty
upkg search --exact git
upkg --self-upgrade
upkg help
```

## Search

`upkg search <query>` routes to the platform's native search:

| Platform | Runs |
|---|---|
| Debian/Ubuntu | `apt search` |
| Fedora/RHEL | `dnf search` |
| Arch | `pacman -Ss` |
| openSUSE | `zypper search` |
| OpenWrt | `opkg find` |
| FreeBSD / DragonFly BSD | `pkg search` |
| Ravenports | `rvn search` |
| Android (Termux) | `pkg search` |
| Windows | `winget search` / `choco search` |
| macOS | built-in search against the Homebrew JSON index |

Output passes through verbatim on native backends so you get the
formatting you're used to. On macOS the engine prints a uniform
tab-separated line: `kind<TAB>name<TAB>version<TAB>description`.

Flags:

- `--app`: on macOS, restrict to casks. No-op (and rejected as
  `Unsupported`) on other platforms.
- `--exact`, `-e`: exact name match. Maps to `pkg/winget/choco/rvn -e`
  and to an anchored `^…$` regex for `pacman`/`opkg`. Returns an
  `Unsupported` error on managers without an exact mode
  (`apt`/`dnf`/`yum`/`zypper`).
- `--refresh`: macOS only. Forces revalidation of the cached Homebrew
  index instead of using the on-disk copy (default TTL 12 hours).

The macOS index lives at `<root>/cache/homebrew-search/` and is
revalidated with `If-None-Match` / `If-Modified-Since`. If the network
or server fails, a stale cached copy is used and a warning is printed
to stderr.

## Build

```bash
cargo build -p upkg
just qq
```
