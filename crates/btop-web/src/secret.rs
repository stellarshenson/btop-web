//! ACC-NET-146-SECRET-INPUT: one loader for every secret the operator
//! supplies, so a secret arrives as a file path or an inherited descriptor
//! and never as a command-line value.
//!
//! `fd:N` names descriptor `N` inherited from the process that started
//! btop-web. The descriptor is read once at startup and closed when the read
//! finishes; it is never reopened by path. Every error names the secret and
//! its source, and a passphrase or key buffer is zeroed when it drops.

use std::io::Read as _;
use std::os::fd::RawFd;
use std::path::PathBuf;

use anyhow::{Context, Result};
use filedescriptor::FileDescriptor;
use zeroize::Zeroizing;

/// Where an operator-supplied secret comes from: a file path, or `fd:N` for a
/// descriptor inherited from the parent process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Path(PathBuf),
    Fd(RawFd),
}

impl Source {
    /// Parse a client `connect_key` source: `fd:N` names a descriptor, any
    /// other non-empty text is a path.
    pub fn parse(text: &str) -> Result<Self, String> {
        if let Some(number) = text.strip_prefix("fd:") {
            return number
                .parse::<RawFd>()
                .ok()
                .filter(|fd| *fd >= 0)
                .map(Self::Fd)
                .ok_or_else(|| {
                    format!("`{text}` is not `fd:N` with N a non-negative descriptor number")
                });
        }
        if text.is_empty() {
            return Err("an empty value names no file and no descriptor".to_string());
        }
        Ok(Self::Path(PathBuf::from(text)))
    }

    /// The source as a phrase an error or a log line can carry.
    pub fn describe(&self) -> String {
        match self {
            Self::Path(path) => format!("file {}", path.display()),
            Self::Fd(fd) => format!("descriptor {fd}"),
        }
    }
}

/// Read a secret once: the file's bytes, or everything on the descriptor,
/// which is closed when the read finishes. An empty source is an error,
/// because an empty key or certificate would let an empty value through.
pub fn read(source: &Source, secret: &str) -> Result<Zeroizing<Vec<u8>>> {
    let bytes = match source {
        Source::Path(path) => std::fs::read(path)
            .map(Zeroizing::new)
            .with_context(|| format!("cannot read {secret} from file {}", path.display()))?,
        Source::Fd(fd) => read_descriptor(*fd)
            .with_context(|| format!("cannot read {secret} from descriptor {fd}"))?,
    };
    anyhow::ensure!(
        !bytes.is_empty(),
        "{secret} from {} is empty",
        source.describe()
    );
    Ok(bytes)
}

/// Read an inherited descriptor to its end. [`FileDescriptor`] takes
/// ownership, so dropping it closes the descriptor: the secret is read once
/// and the channel is gone.
fn read_descriptor(fd: RawFd) -> Result<Zeroizing<Vec<u8>>> {
    let mut descriptor = FileDescriptor::new(fd);
    let mut bytes = Zeroizing::new(Vec::new());
    descriptor.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Where an encrypted key's passphrase comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassphraseSource {
    /// The value of the named environment variable.
    Env(String),
    /// A file path or `fd:N`.
    File(Source),
}

impl PassphraseSource {
    /// The source as a phrase an error or a log line can carry.
    pub fn describe(&self) -> String {
        match self {
            Self::Env(name) => format!("environment variable {name}"),
            Self::File(source) => source.describe(),
        }
    }
}

/// Read the passphrase once, without a trailing line ending from an editor or
/// `echo`, and zero it when the buffer drops.
pub fn passphrase(source: &PassphraseSource) -> Result<Zeroizing<Vec<u8>>> {
    let mut bytes = match source {
        PassphraseSource::Env(name) => {
            let value = std::env::var_os(name).ok_or_else(|| {
                anyhow::anyhow!("TLS key passphrase: environment variable {name} is not set")
            })?;
            use std::os::unix::ffi::OsStringExt as _;
            Zeroizing::new(value.into_vec())
        }
        PassphraseSource::File(source) => read(source, "TLS key passphrase")?,
    };
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    anyhow::ensure!(
        !bytes.is_empty(),
        "TLS key passphrase from {} is empty",
        source.describe()
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::os::fd::IntoRawFd as _;

    /// A flag or file value is a path unless it is exactly `fd:N`.
    #[test]
    fn sources_parse_paths_and_descriptors() {
        assert_eq!(
            Source::parse("/etc/btop-web/key").unwrap(),
            Source::Path(PathBuf::from("/etc/btop-web/key"))
        );
        assert_eq!(Source::parse("fd:3").unwrap(), Source::Fd(3));
        assert!(Source::parse("fd:-1").is_err());
        assert!(Source::parse("fd:x").is_err());
        assert!(Source::parse("").is_err());
    }

    /// ACC-NET-146-SECRET-INPUT: a descriptor carries the secret, is read to
    /// its end, and is closed afterwards, so a second read cannot succeed.
    #[test]
    fn a_descriptor_carries_the_secret_once() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(b"first-").unwrap();
        writer.write_all(b"second").unwrap();
        drop(writer);
        let fd = reader.into_raw_fd();
        let bytes = read(&Source::Fd(fd), "test secret").unwrap();
        assert_eq!(&*bytes, b"first-second");
        let error = format!("{:#}", read(&Source::Fd(fd), "test secret").unwrap_err());
        assert!(error.contains("test secret"), "{error}");
        assert!(error.contains(&format!("descriptor {fd}")), "{error}");
    }

    /// An empty descriptor stops the read with the secret and the descriptor
    /// number named, never as an empty value.
    #[test]
    fn an_empty_descriptor_is_refused() {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(writer);
        let fd = reader.into_raw_fd();
        let error = format!("{:#}", read(&Source::Fd(fd), "access key").unwrap_err());
        assert!(error.contains("access key"), "{error}");
        assert!(error.contains(&format!("descriptor {fd}")), "{error}");
        assert!(error.contains("empty"), "{error}");
    }

    /// A closed descriptor is EBADF, and the error still names the secret and
    /// the number that was given.
    #[test]
    fn a_closed_descriptor_names_the_secret_and_number() {
        let error = format!("{:#}", read(&Source::Fd(99), "TLS key").unwrap_err());
        assert!(error.contains("TLS key"), "{error}");
        assert!(error.contains("descriptor 99"), "{error}");
    }

    /// A missing file names the secret and the path.
    #[test]
    fn a_missing_file_names_the_secret_and_path() {
        let source = Source::Path(PathBuf::from("/nonexistent/btop-web-secret"));
        let error = format!("{:#}", read(&source, "TLS certificate").unwrap_err());
        assert!(error.contains("TLS certificate"), "{error}");
        assert!(error.contains("/nonexistent/btop-web-secret"), "{error}");
    }

    /// A missing environment variable is refused by name, never read as an
    /// empty passphrase.
    #[test]
    fn a_missing_passphrase_variable_is_named() {
        let source = PassphraseSource::Env("BTOP_WEB_TEST_UNSET_PASSPHRASE".to_string());
        let error = format!(
            "{:#}",
            passphrase(&source).expect_err("an unset variable is not a passphrase")
        );
        assert!(error.contains("TLS key passphrase"), "{error}");
        assert!(error.contains("BTOP_WEB_TEST_UNSET_PASSPHRASE"), "{error}");
    }

    /// A passphrase file's trailing newline is not part of the passphrase,
    /// and the value is returned without it.
    #[test]
    fn a_passphrase_file_drops_the_line_ending() {
        let path = std::env::temp_dir().join(format!(
            "btop-web-passphrase-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, b"correct horse\n").unwrap();
        let source = PassphraseSource::File(Source::Path(path.clone()));
        let bytes = passphrase(&source).unwrap();
        assert_eq!(&*bytes, b"correct horse");
        let _ = std::fs::remove_file(&path);
    }
}
