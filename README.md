# ssh-copy-id for Windows

[![CI](https://github.com/netkeyok/ssh-copy-id-windows/actions/workflows/ci.yml/badge.svg)](https://github.com/netkeyok/ssh-copy-id-windows/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/netkeyok/ssh-copy-id-windows?display_name=tag)](https://github.com/netkeyok/ssh-copy-id-windows/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A native Windows build of `ssh-copy-id`, written in Rust with no third-party
crates. It installs your public key into `~/.ssh/authorized_keys` on a remote
host so that `ssh user@host` stops asking for a password.

```powershell
ssh-copy-id user@192.168.1.10
```

## Why

Windows ships `ssh.exe` and `ssh-keygen.exe`, but not `ssh-copy-id`: the real
one is a POSIX shell script, so PowerShell answers with

```
ssh-copy-id: Термин "ssh-copy-id" не распознается как имя ...
```

This project replaces the script with a single self-contained `.exe`
(~250 KB, no runtime dependencies, no Git Bash required).

## Install

### One-liner (recommended)

```powershell
irm https://raw.githubusercontent.com/netkeyok/ssh-copy-id-windows/main/install.ps1 | iex
```

The script downloads the latest release, installs it to
`%LOCALAPPDATA%\Programs\ssh-copy-id`, adds that folder to your user `PATH`
and makes the command available in the current session. Open a new terminal
afterwards.

Options for the installer:

| Parameter | Meaning |
| --- | --- |
| `-Version 0.1.0` | install a specific release instead of the latest |
| `-InstallDir <path>` | install somewhere else |
| `-NoPathUpdate` | do not touch `PATH` |

Downloaded and reviewed the script first? Run it locally:

```powershell
.\install.ps1
```

### Manual

1. Download `ssh-copy-id.exe` from the [latest release](https://github.com/netkeyok/ssh-copy-id-windows/releases/latest).
2. Put it in a folder on your `PATH`, for example `%LOCALAPPDATA%\Programs\ssh-copy-id`.

### From source

```powershell
cargo install --git https://github.com/netkeyok/ssh-copy-id-windows
```

## Usage

```powershell
ssh-copy-id [options] [user@]host
```

```
  -i <file>       key to install (.pub or private key); may be repeated
  -p <port>       SSH port
  -o <option>     extra ssh option, e.g. -o StrictHostKeyChecking=no
  -n              show what would be installed, change nothing
  -f              skip the already-installed check, append anyway
  -t <shell>      ignored (OpenSSH compatibility)
  -h, --help      this help
  -V, --version   version
```

Without `-i` the standard keys from `%USERPROFILE%\.ssh` are used:
`id_ed25519.pub`, `id_ecdsa.pub`, `id_rsa.pub`, `id_dsa.pub`.

Examples:

```powershell
# copy the default key(s)
ssh-copy-id user@192.168.1.10

# custom port and key
ssh-copy-id -p 2222 -i ~/.ssh/id_ed25519.pub user@192.168.1.10

# see what would happen, without connecting
ssh-copy-id -n user@192.168.1.10
```

Then verify:

```powershell
ssh user@192.168.1.10
```

## How it works

1. Collects public keys locally and removes duplicates.
2. Reads the current `~/.ssh/authorized_keys` from the host:
   `ssh host "sh -c 'cat ~/.ssh/authorized_keys 2>/dev/null || true'"`.
3. Compares keys by `type + base64` and ignores the trailing comment, so
   running the command twice does not append the same key again.
4. Sends the missing keys on stdin of
   `sh -c 'umask 077; mkdir -p ~/.ssh && chmod 700 ~/.ssh; cat >> ~/.ssh/authorized_keys; chmod 600 ~/.ssh/authorized_keys'`.

Keys travel over stdin, exactly like in the original script, so Windows
quoting never gets in the way. The password prompt is read from the console,
not from stdin, so it still works interactively. The `sh -c` wrapper keeps the
command working when the remote login shell is `csh`/`tcsh`.

## Differences from the OpenSSH script

- Native `.exe`: no `sh`, `bash`, MSYS2 or Git Bash needed.
- `-i` accepts a private key path; the public part is taken from the sibling
  `.pub` file or derived with `ssh-keygen -y`.
- Duplicates are detected by key material, not by the whole line, so a
  different comment does not create a second entry.
- If the remote `authorized_keys` lacks a trailing newline, one is inserted
  before the new key.
- Prints which file each installed key came from.

## Building and testing

```powershell
cargo test
cargo build --release
.\target\release\ssh-copy-id.exe --version
```

Releases are built by GitHub Actions on every `v*` tag; the workflow also
publishes a SHA-256 checksum next to the executable.

## Uninstall

```powershell
irm https://raw.githubusercontent.com/netkeyok/ssh-copy-id-windows/main/uninstall.ps1 | iex
```

Or delete `%LOCALAPPDATA%\Programs\ssh-copy-id` and remove that entry from
your user `PATH`.

## License

[MIT](LICENSE)
