//! ACC-NET-144-ACCESS-KEY: the agent's access key, drawn once and kept in the
//! config directory.
//!
//! The key is 32 bytes from the operating system CSPRNG, written as base64url
//! without padding with mode 0600 on the start that creates it. A later start
//! reads that file and never rewrites it, so the one terminal line on the
//! creating start is the only time the key is shown.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::config::config_base_in;
/// The bytes drawn from the OS CSPRNG; 32 bytes are 43 base64url characters.
const KEY_BYTES: usize = 32;

/// The agent's key file: `$XDG_CONFIG_HOME/btop-web/access-key` when
/// XDG_CONFIG_HOME names an absolute directory, else
/// `$HOME/.config/btop-web/access-key`; `None` when neither is set.
pub fn key_path() -> Option<PathBuf> {
    key_path_in(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

fn key_path_in(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    Some(
        config_base_in(xdg, home)?
            .join("btop-web")
            .join("access-key"),
    )
}

/// The agent's key and whether this start created it.
///
/// The creating start draws 32 bytes from the OS CSPRNG and writes them with
/// mode 0600; a start that finds the file reads it and leaves it untouched.
pub fn load_or_create(path: &Path) -> std::io::Result<(String, bool)> {
    match read_key(path) {
        Ok(key) => Ok((key, false)),
        Err(error) if error.kind() == ErrorKind::NotFound => create_key(path),
        Err(error) => Err(error),
    }
}

/// The file's one line, without its line ending. An empty file is an error
/// rather than an empty key, which would let an empty header through.
fn read_key(path: &Path) -> std::io::Result<String> {
    let key = std::fs::read_to_string(path)?.trim().to_string();
    if key.is_empty() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "access key file is empty",
        ));
    }
    Ok(key)
}

/// ACC-NET-144-ACCESS-KEY: show a newly created key on the controlling
/// terminal, the only place it is ever printed. The line names the host, the
/// key and the file, and says the key is shown once. A start with no
/// controlling terminal, or one whose write fails, prints nothing and returns
/// `false`, so the caller logs the file path alone; stdout and stderr never
/// carry the key.
pub fn announce_created(path: &Path, key: &str) -> bool {
    let Ok(mut tty) = OpenOptions::new().write(true).open("/dev/tty") else {
        return false;
    };
    let host = nix::unistd::gethostname()
        .ok()
        .and_then(|name| name.into_string().ok())
        .unwrap_or_else(|| "this host".to_string());
    writeln!(
        tty,
        "btop-web: new access key for {host}: {key} (shown once, stored in {})",
        path.display()
    )
    .is_ok()
}

/// ACC-NET-146-SECRET-INPUT: the key text from an operator-supplied file or
/// descriptor, trimmed. Empty or non-UTF-8 text is an error, never an empty
/// key.
pub fn key_from_bytes(bytes: &[u8]) -> anyhow::Result<String> {
    let key = std::str::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("the access key is not UTF-8 text"))?
        .trim();
    anyhow::ensure!(!key.is_empty(), "the access key is empty");
    Ok(key.to_string())
}

fn create_key(path: &Path) -> std::io::Result<(String, bool)> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut bytes = [0u8; KEY_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| std::io::Error::other(error.to_string()))?;
    let key = URL_SAFE_NO_PAD.encode(bytes);
    // create_new: a concurrent first start reads the key it lost the race to
    // instead of overwriting it.
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            return Ok((read_key(path)?, false));
        }
        Err(error) => return Err(error),
    };
    file.write_all(format!("{key}\n").as_bytes())?;
    file.sync_all()?;
    // The open mode is filtered by umask; set the bits after the write so the
    // file is 0600 whatever the umask was.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok((key, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("btop-web-access-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// ACC-NET-144-ACCESS-KEY: the first start draws a 32-byte key, writes it
    /// 0600, and a later start returns the same bytes without rewriting them.
    #[test]
    fn the_key_is_created_once_with_mode_0600() {
        let dir = temp_dir("create");
        let path = dir.join("access-key");
        let (key, created) = load_or_create(&path).unwrap();
        assert!(created);
        assert_eq!(key.len(), 43, "32 bytes are 43 base64url characters");
        assert!(
            key.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "{key}"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let written = std::fs::read(&path).unwrap();

        let (again, created) = load_or_create(&path).unwrap();
        assert!(!created);
        assert_eq!(again, key);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            written,
            "the file was rewritten"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A configured directory wins over the home default only when it is
    /// absolute, the same rule the web config file follows.
    #[test]
    fn the_key_lives_in_the_config_directory() {
        assert_eq!(
            key_path_in(Some("/tmp/xdg".into()), Some("/home/x".into())),
            Some(PathBuf::from("/tmp/xdg/btop-web/access-key"))
        );
        assert_eq!(
            key_path_in(Some("relative".into()), Some("/home/x".into())),
            Some(PathBuf::from("/home/x/.config/btop-web/access-key"))
        );
        assert_eq!(
            key_path_in(None, Some("/home/x".into())),
            Some(PathBuf::from("/home/x/.config/btop-web/access-key"))
        );
        assert_eq!(key_path_in(None, None), None);
    }

    /// An empty file is refused, never accepted as an empty key.
    #[test]
    fn an_empty_key_file_is_refused() {
        let dir = temp_dir("empty");
        let path = dir.join("access-key");
        std::fs::write(&path, "\n").unwrap();
        assert!(load_or_create(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
