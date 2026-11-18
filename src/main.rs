//! `ssh-copy-id` for Windows.
//!
//! A native port of the OpenSSH shell script of the same name: it installs
//! your public key (or several) into `~/.ssh/authorized_keys` on a remote
//! host so that key-based login starts working.
//!
//! Windows ships `ssh.exe` and `ssh-keygen.exe` but not the `ssh-copy-id`
//! script, which is a POSIX shell script. This binary fills that gap.
//!
//! No third-party crates: only `std` plus the system `ssh.exe` /
//! `ssh-keygen.exe`.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Keys copied when `-i` is not given (same order as OpenSSH).
const DEFAULT_KEY_NAMES: [&str; 4] = ["id_ed25519", "id_ecdsa", "id_rsa", "id_dsa"];

/// Reads the current `authorized_keys` on the remote host.
///
/// Wrapped in `sh -c` so a `csh`/`tcsh` login shell does not choke on
/// POSIX redirections.
const REMOTE_CHECK: &str = "sh -c 'cat ~/.ssh/authorized_keys 2>/dev/null || true'";

/// Installs keys: they arrive on this command's stdin.
const REMOTE_INSTALL: &str = "sh -c 'umask 077; mkdir -p ~/.ssh && chmod 700 ~/.ssh || exit 1; \
cat >> ~/.ssh/authorized_keys || exit 1; chmod 600 ~/.ssh/authorized_keys'";

// ---------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Options {
    identities: Vec<String>,
    port: Option<String>,
    ssh_options: Vec<String>,
    dry_run: bool,
    force: bool,
    target: String,
}

enum Parsed {
    Run(Options),
    Help,
    Version,
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut opts = Options::default();
    let mut positional: Vec<String> = Vec::new();
    let mut args = args.into_iter();
    let mut no_more_opts = false;

    while let Some(arg) = args.next() {
        if no_more_opts {
            positional.push(arg);
            continue;
        }
        if arg == "--" {
            no_more_opts = true;
            continue;
        }
        if arg == "--help" {
            return Ok(Parsed::Help);
        }
        if arg == "--version" {
            return Ok(Parsed::Version);
        }
        if arg.starts_with("--") {
            return Err(format!("unknown option: {arg}"));
        }
        if arg.starts_with('-') && arg.len() > 1 {
            let chars: Vec<char> = arg.chars().skip(1).collect();
            let mut idx = 0usize;
            while idx < chars.len() {
                let flag = chars[idx];
                idx += 1;
                match flag {
                    'h' => return Ok(Parsed::Help),
                    'V' => return Ok(Parsed::Version),
                    'n' => opts.dry_run = true,
                    'f' => opts.force = true,
                    'p' | 'i' | 'o' | 't' => {
                        let attached: String = chars[idx..].iter().collect();
                        let value = if attached.is_empty() {
                            match args.next() {
                                Some(value) => value,
                                None => return Err(format!("option -{flag} requires a value")),
                            }
                        } else {
                            idx = chars.len();
                            attached
                        };
                        match flag {
                            'p' => opts.port = Some(value),
                            'i' => opts.identities.push(value),
                            'o' => opts.ssh_options.push(value),
                            // -t target_shell: deprecated in OpenSSH, accepted for compat.
                            _ => {}
                        }
                    }
                    _ => return Err(format!("unknown option: -{flag}")),
                }
            }
            continue;
        }
        positional.push(arg);
    }

    match positional.len() {
        0 => Err("no target host given (for example: ssh-copy-id user@host)".to_string()),
        1 => {
            opts.target = positional.remove(0);
            Ok(Parsed::Run(opts))
        }
        _ => Err(format!("unexpected extra arguments: {}", positional[1..].join(" "))),
    }
}

fn usage() -> String {
    format!(
        "ssh-copy-id {VERSION} - install a public SSH key on a remote host (Windows)

Usage:
  ssh-copy-id [options] [user@]host

Options:
  -i <file>       key to install (.pub or private key); may be repeated
  -p <port>       SSH port
  -o <option>     extra ssh option, e.g. -o StrictHostKeyChecking=no
  -n              show what would be installed, change nothing
  -f              skip the already-installed check, append anyway
  -t <shell>      ignored (OpenSSH compatibility)
  -h, --help      this help
  -V, --version   version

Without -i the standard keys from %USERPROFILE%\\.ssh are used:
  id_ed25519.pub, id_ecdsa.pub, id_rsa.pub, id_dsa.pub

Examples:
  ssh-copy-id user@192.168.1.10
  ssh-copy-id -p 2222 -i C:\\Users\\me\\.ssh\\id_ed25519.pub user@192.168.1.10"
    )
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

struct Key {
    source: PathBuf,
    line: String,
}

/// Key identity: "type + base64" without the trailing comment.
/// This is what decides whether the key is already installed.
fn key_blob_id(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    let kind = parts.next()?;
    let blob = parts.next()?;
    if kind.is_empty() || blob.is_empty() {
        return None;
    }
    Some(format!("{kind} {blob}"))
}

fn parse_key_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| key_blob_id(line).is_some())
        .map(str::to_string)
        .collect()
}

fn home_dir() -> PathBuf {
    if let Some(path) = env::var_os("USERPROFILE") {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("HOME") {
        return PathBuf::from(path);
    }
    PathBuf::from(".")
}

/// Joins a relative path onto a base, normalising separators so that
/// `~/.ssh/key` becomes `C:\Users\...\.ssh\key` on Windows.
fn join_normalized(base: PathBuf, rest: &str) -> PathBuf {
    let mut path = base;
    for part in rest.split(['/', '\\']) {
        if !part.is_empty() {
            path.push(part);
        }
    }
    path
}

fn expand_tilde(spec: &str) -> PathBuf {
    if spec == "~" {
        return home_dir();
    }
    if let Some(rest) = spec.strip_prefix("~/").or_else(|| spec.strip_prefix("~\\")) {
        return join_normalized(home_dir(), rest);
    }
    PathBuf::from(spec)
}

fn is_pub_extension(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.eq_ignore_ascii_case("pub"))
        .unwrap_or(false)
}

fn with_pub_suffix(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.pub", path.display()))
}

fn read_pub_keys(path: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let keys = parse_key_lines(&text);
    if keys.is_empty() {
        return Err(format!("no public key found in {}", path.display()));
    }
    Ok(keys)
}

/// Derives the public key from a private key file when no `.pub` sits next to it.
fn derive_public_key(private: &Path) -> Result<String, String> {
    let output = Command::new("ssh-keygen")
        .arg("-y")
        .arg("-f")
        .arg(private)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("failed to run ssh-keygen: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "ssh-keygen could not derive a public key from {}: {}",
            private.display(),
            stderr.trim()
        ));
    }
    let derived = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if derived.is_empty() {
        return Err(format!(
            "ssh-keygen returned an empty public key for {}",
            private.display()
        ));
    }
    Ok(derived)
}

fn keys_from_identity(spec: &str) -> Result<Vec<(PathBuf, String)>, String> {
    let path = expand_tilde(spec);

    if path.is_file() {
        if is_pub_extension(&path) {
            return Ok(read_pub_keys(&path)?
                .into_iter()
                .map(|line| (path.clone(), line))
                .collect());
        }
        let pub_path = with_pub_suffix(&path);
        if pub_path.is_file() {
            return Ok(read_pub_keys(&pub_path)?
                .into_iter()
                .map(|line| (pub_path.clone(), line))
                .collect());
        }
        let derived = derive_public_key(&path)?;
        return Ok(vec![(path, derived)]);
    }

    let pub_path = with_pub_suffix(&path);
    if pub_path.is_file() {
        return Ok(read_pub_keys(&pub_path)?
            .into_iter()
            .map(|line| (pub_path.clone(), line))
            .collect());
    }

    Err(format!("key file not found: {}", path.display()))
}

fn collect_keys(opts: &Options) -> Result<Vec<Key>, String> {
    let mut found: Vec<Key> = Vec::new();

    if !opts.identities.is_empty() {
        for spec in &opts.identities {
            for (source, line) in keys_from_identity(spec)? {
                found.push(Key { source, line });
            }
        }
    } else {
        let ssh_dir = home_dir().join(".ssh");
        for name in DEFAULT_KEY_NAMES {
            let pub_path = ssh_dir.join(format!("{name}.pub"));
            if pub_path.is_file() {
                for line in read_pub_keys(&pub_path)? {
                    found.push(Key {
                        source: pub_path.clone(),
                        line,
                    });
                }
            }
        }
        if found.is_empty() {
            return Err(format!(
                "no public key found in {}\n\
                 create one:        ssh-keygen -t ed25519\n\
                 or point to a key: ssh-copy-id -i path\\to\\key.pub <host>",
                ssh_dir.display()
            ));
        }
    }

    let mut seen: HashSet<String> = HashSet::new();
    found.retain(|key| match key_blob_id(&key.line) {
        Some(id) => seen.insert(id),
        None => false,
    });

    Ok(found)
}

// ---------------------------------------------------------------------------
// Talking to the remote host
// ---------------------------------------------------------------------------

fn ssh_command(opts: &Options) -> Command {
    let mut cmd = Command::new("ssh");
    for option in &opts.ssh_options {
        cmd.arg("-o").arg(option);
    }
    if let Some(port) = &opts.port {
        cmd.arg("-p").arg(port);
    }
    cmd.arg(&opts.target);
    cmd
}

fn describe_command(opts: &Options, remote: &str) -> String {
    let mut text = String::from("ssh");
    for option in &opts.ssh_options {
        text.push_str(&format!(" -o {option}"));
    }
    if let Some(port) = &opts.port {
        text.push_str(&format!(" -p {port}"));
    }
    text.push(' ');
    text.push_str(&opts.target);
    text.push(' ');
    text.push_str(remote);
    text
}

/// Returns the current content of `~/.ssh/authorized_keys` on the host.
fn fetch_existing(opts: &Options) -> Result<Vec<u8>, String> {
    let output = ssh_command(opts)
        .arg(REMOTE_CHECK)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|err| format!("failed to run ssh: {err}"))?;

    if !output.status.success() {
        return Err(match output.status.code() {
            Some(code) => format!(
                "ssh exited with code {code}; could not check access to {}",
                opts.target
            ),
            None => format!(
                "ssh terminated abnormally while connecting to {}",
                opts.target
            ),
        });
    }

    Ok(output.stdout)
}

fn install_keys(opts: &Options, payload: &[u8]) -> Result<(), String> {
    let mut child = ssh_command(opts)
        .arg(REMOTE_INSTALL)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| format!("failed to run ssh: {err}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(payload)
            .map_err(|err| format!("failed to send keys to ssh: {err}"))?;
    }

    let status = child
        .wait()
        .map_err(|err| format!("failed waiting for ssh: {err}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(match status.code() {
            Some(code) => format!("ssh exited with code {code}; key was not installed"),
            None => "ssh terminated abnormally; key was not installed".to_string(),
        })
    }
}

/// Builds the data for the remote command's stdin.
///
/// If the existing `authorized_keys` does not end with a newline, add one
/// first, otherwise the new key would be glued to the last line.
fn build_payload(existing: &[u8], keys: &[&Key]) -> Vec<u8> {
    let mut payload = Vec::new();
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        payload.push(b'\n');
    }
    for key in keys {
        payload.extend_from_slice(key.line.as_bytes());
        payload.push(b'\n');
    }
    payload
}

fn short_id(line: &str) -> String {
    let mut parts = line.split_whitespace();
    let kind = parts.next().unwrap_or("?");
    let blob = parts.next().unwrap_or("");
    let tail: String = blob
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    format!("{kind} ...{tail}")
}

fn run(opts: Options) -> Result<(), String> {
    let keys = collect_keys(&opts)?;

    if opts.dry_run {
        println!("Target: {}", opts.target);
        println!("Keys ({}):", keys.len());
        for key in &keys {
            println!("  {} - {}", key.source.display(), short_id(&key.line));
        }
        println!("Command: {}", describe_command(&opts, REMOTE_INSTALL));
        return Ok(());
    }

    let existing = if opts.force {
        Vec::new()
    } else {
        fetch_existing(&opts)?
    };

    let installed: HashSet<String> = parse_key_lines(&String::from_utf8_lossy(&existing))
        .iter()
        .filter_map(|line| key_blob_id(line))
        .collect();

    let missing: Vec<&Key> = keys
        .iter()
        .filter(|key| match key_blob_id(&key.line) {
            Some(id) => !installed.contains(&id),
            None => false,
        })
        .collect();

    if missing.is_empty() {
        println!(
            "Key(s) already present in ~/.ssh/authorized_keys on {}; nothing to do.",
            opts.target
        );
        return Ok(());
    }

    let payload = build_payload(&existing, &missing);
    install_keys(&opts, &payload)?;

    for key in &missing {
        println!("installed key: {}", key.source.display());
    }
    println!(
        "Done: {}. Verify with:  ssh {}",
        opts.target, opts.target
    );
    Ok(())
}

fn main() {
    match parse_args(env::args().skip(1)) {
        Ok(Parsed::Help) => println!("{}", usage()),
        Ok(Parsed::Version) => println!("ssh-copy-id {VERSION}"),
        Ok(Parsed::Run(opts)) => {
            if let Err(err) = run(opts) {
                eprintln!("ssh-copy-id: {err}");
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("ssh-copy-id: {err}");
            eprintln!();
            eprintln!("{}", usage());
            std::process::exit(2);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Parsed, String> {
        parse_args(args.iter().map(|value| value.to_string()))
    }

    fn run_opts(args: &[&str]) -> Options {
        match parse(args).expect("arguments should parse") {
            Parsed::Run(opts) => opts,
            _ => panic!("expected the run mode"),
        }
    }

    fn key(line: &str) -> Key {
        Key {
            source: PathBuf::from("test.pub"),
            line: line.to_string(),
        }
    }

    #[test]
    fn parses_target_only() {
        let opts = run_opts(&["user@192.168.3.206"]);
        assert_eq!(opts.target, "user@192.168.3.206");
        assert!(!opts.dry_run);
        assert!(!opts.force);
        assert!(opts.identities.is_empty());
    }

    #[test]
    fn parses_attached_and_separate_values() {
        let opts = run_opts(&[
            "-p",
            "2222",
            "-i~/.ssh/id_ed25519.pub",
            "-o",
            "ConnectTimeout=5",
            "host",
        ]);
        assert_eq!(opts.port.as_deref(), Some("2222"));
        assert_eq!(opts.identities, vec!["~/.ssh/id_ed25519.pub".to_string()]);
        assert_eq!(opts.ssh_options, vec!["ConnectTimeout=5".to_string()]);
        assert_eq!(opts.target, "host");
    }

    #[test]
    fn parses_clustered_flags() {
        let opts = run_opts(&["-nf", "host"]);
        assert!(opts.dry_run);
        assert!(opts.force);
    }

    #[test]
    fn parses_double_dash_terminator() {
        let opts = run_opts(&["-n", "--", "-weird-host"]);
        assert_eq!(opts.target, "-weird-host");
    }

    #[test]
    fn help_and_version_win_over_target() {
        assert!(matches!(parse(&["--help", "host"]), Ok(Parsed::Help)));
        assert!(matches!(parse(&["-V"]), Ok(Parsed::Version)));
    }

    #[test]
    fn rejects_unknown_option() {
        let err = parse(&["-Z", "host"]).err().expect("expected an error");
        assert!(err.contains("-Z"), "message: {err}");
    }

    #[test]
    fn rejects_missing_value() {
        assert!(parse(&["-p"]).is_err());
    }

    #[test]
    fn requires_target() {
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn rejects_extra_positional() {
        assert!(parse(&["a", "b"]).is_err());
    }

    #[test]
    fn blob_id_ignores_comment_and_spacing() {
        let line = "ssh-ed25519  AAAAC3NzaC1lZDI1NTE5AAAAIAbc  user@example";
        assert_eq!(
            key_blob_id(line).as_deref(),
            Some("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAbc")
        );
        assert!(key_blob_id("   ").is_none());
        assert!(key_blob_id("only-one-field").is_none());
    }

    #[test]
    fn parses_key_file_with_comments_and_blanks() {
        let text = "# comment\n\nssh-ed25519 AAAA1 a@b\n  \nssh-rsa AAAA2\n";
        assert_eq!(parse_key_lines(text).len(), 2);
    }

    #[test]
    fn payload_keeps_file_ending_with_newline() {
        let k = key("ssh-ed25519 AAAA1");
        let payload = build_payload(b"ssh-rsa OLD\n", &[&k]);
        assert_eq!(payload, b"ssh-ed25519 AAAA1\n");
    }

    #[test]
    fn payload_fixes_missing_trailing_newline() {
        let k = key("ssh-ed25519 AAAA1");
        let payload = build_payload(b"ssh-rsa OLD", &[&k]);
        assert_eq!(payload, b"\nssh-ed25519 AAAA1\n");
    }

    #[test]
    fn payload_for_empty_existing_file() {
        let k = key("ssh-ed25519 AAAA1");
        assert_eq!(build_payload(b"", &[&k]), b"ssh-ed25519 AAAA1\n");
    }

    #[test]
    fn remote_commands_use_posix_shell_and_single_quotes_only() {
        for command in [REMOTE_CHECK, REMOTE_INSTALL] {
            assert!(command.starts_with("sh -c '"), "command: {command}");
            assert!(command.ends_with('\''), "command: {command}");
            assert_eq!(
                command.matches('\'').count(),
                2,
                "no single quotes inside: {command}"
            );
            assert!(!command.contains('"'), "command: {command}");
        }
    }

    #[test]
    fn expand_tilde_uses_home() {
        let home = home_dir();
        assert_eq!(expand_tilde("~/x"), home.join("x"));
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(
            expand_tilde("~/.ssh/id_ed25519.pub"),
            home.join(".ssh").join("id_ed25519.pub")
        );
        assert_eq!(expand_tilde("C:\\temp\\k"), PathBuf::from("C:\\temp\\k"));
    }
}
