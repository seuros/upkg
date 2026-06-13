#!/bin/sh
set -eu

REPO="seuros/upkg"
INSTALL_DIR="${UPKG_INSTALL_DIR:-/usr/local/bin}"
BINARY="upkg"

main() {
    need_cmd curl
    need_cmd tar
    need_cmd uname

    arch="$(uname -m)"
    os="$(uname -s)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64|amd64) target="x86_64-unknown-linux-gnu" ;;
                aarch64|arm64) target="aarch64-unknown-linux-gnu" ;;
                *) err "unsupported architecture: $arch" ;;
            esac
            # prefer musl on alpine/containers
            if [ -f /etc/alpine-release ] || ! ldd --version >/dev/null 2>&1; then
                case "$arch" in
                    x86_64|amd64) target="x86_64-unknown-linux-musl" ;;
                esac
            fi
            ;;
        Darwin)
            case "$arch" in
                x86_64) target="x86_64-apple-darwin" ;;
                arm64)  target="aarch64-apple-darwin" ;;
                *) err "unsupported architecture: $arch" ;;
            esac
            ;;
        *) err "unsupported OS: $os (supported: Linux, macOS)" ;;
    esac

    say "detected target: $target"

    tag="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' | head -1 | cut -d'"' -f4)"
    [ -z "$tag" ] && err "failed to fetch latest release tag"

    version="${tag#upkg-}"
    say "latest release: $version"

    archive="${tag}-${target}.tar.gz"
    url="https://github.com/${REPO}/releases/download/${tag}/${archive}"

    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    say "downloading $url"
    curl -fSL --progress-bar -o "$tmpdir/$archive" "$url"

    say "extracting to $INSTALL_DIR"
    tar xzf "$tmpdir/$archive" -C "$tmpdir"

    if [ ! -d "$INSTALL_DIR" ]; then
        if mkdir -p "$INSTALL_DIR" 2>/dev/null; then
            :
        else
            say "elevated permissions required to create $INSTALL_DIR"
            sudo mkdir -p "$INSTALL_DIR"
        fi
    fi

    if [ -w "$INSTALL_DIR" ]; then
        install -m 755 "$tmpdir/$BINARY" "$INSTALL_DIR/$BINARY"
    else
        say "elevated permissions required to install to $INSTALL_DIR"
        sudo install -m 755 "$tmpdir/$BINARY" "$INSTALL_DIR/$BINARY"
    fi

    say "installed $BINARY $version to $INSTALL_DIR/$BINARY"

    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        say ""
        add_to_path "$INSTALL_DIR"
    fi
}

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        err "required command not found: $1"
    fi
}

say() {
    printf '%s\n' "$1"
}

err() {
    say "error: $1" >&2
    exit 1
}

add_to_path() {
    dir="$1"

    if [ "${UPKG_NO_MODIFY_PATH:-}" = "1" ]; then
        say "WARNING: $dir is not in your PATH"
        say "add it with:  export PATH=\"$dir:\$PATH\""
        return
    fi

    if [ -z "${HOME:-}" ]; then
        say "WARNING: $dir is not in your PATH"
        say "add it with:  export PATH=\"$dir:\$PATH\""
        return
    fi

    profile="$(profile_file)"
    shell_name="${SHELL:-}"
    shell_name="${shell_name##*/}"
    marker="# upkg PATH"

    if [ -f "$profile" ] && grep -Fq "$marker" "$profile" && grep -Fq "$dir" "$profile"; then
        say "$dir is already configured in $profile"
    else
        profile_dir="${profile%/*}"
        [ -d "$profile_dir" ] || mkdir -p "$profile_dir" || {
            say "WARNING: $dir is not in your PATH"
            say "failed to create $profile_dir"
            say "add it with:  export PATH=\"$dir:\$PATH\""
            return
        }

        {
            printf '\n%s\n' "$marker"
            case "$shell_name" in
                fish)
                    printf 'if not contains "%s" $PATH\n' "$dir"
                    printf '    set -gx PATH "%s" $PATH\n' "$dir"
                    printf 'end\n'
                    ;;
                *)
                    printf 'case ":$PATH:" in\n'
                    printf '    *":%s:"*) ;;\n' "$dir"
                    printf '    *) export PATH="%s:$PATH" ;;\n' "$dir"
                    printf 'esac\n'
                    ;;
            esac
        } >> "$profile" || {
            say "WARNING: $dir is not in your PATH"
            say "failed to update $profile"
            say "add it with:  export PATH=\"$dir:\$PATH\""
            return
        }

        say "added $dir to PATH in $profile"
    fi

    case "$shell_name" in
        fish) say "restart your shell to use $BINARY from anywhere" ;;
        *)    say "restart your shell or run:  . \"$profile\"" ;;
    esac
}

profile_file() {
    shell_name="${SHELL:-}"
    shell_name="${shell_name##*/}"

    case "$shell_name" in
        zsh)  printf '%s\n' "$HOME/.zshrc" ;;
        bash) printf '%s\n' "$HOME/.bashrc" ;;
        fish) printf '%s\n' "$HOME/.config/fish/config.fish" ;;
        *)    printf '%s\n' "$HOME/.profile" ;;
    esac
}

main "$@"
