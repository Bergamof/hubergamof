//! `hubergamof install-extension`: puts the Teams browser extension (embedded
//! in the binary) in the user's data directory and tells the browser where the
//! native messaging host is.
//!
//! The extension itself is loaded once by the user ("load unpacked"): the
//! browser starts the host from the manifest written here, which names the
//! extension's own origin and nothing else.

use std::ffi::OsString;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::ValueEnum;
use rust_embed::RustEmbed;
use serde_json::json;
use sha2::{Digest as _, Sha256};

/// Name the extension passes to `connectNative` and the manifest declares.
pub const NATIVE_HOST_NAME: &str = "com.hubergamof.teams";

/// Prefix of the origin a browser passes to a native messaging host.
const BROWSER_ORIGIN_PREFIX: &str = "chrome-extension://";

/// Bytes of the key's hash that make an extension id (32 letters).
const EXTENSION_ID_HASH_BYTES: usize = 16;

/// Folder, under the data directory, that holds the extension.
const EXTENSION_FOLDER: &str = "hubergamof/extension";

/// The extension's files (`ui/teams-extension/`, without its tests).
#[derive(RustEmbed)]
#[folder = "../../ui/teams-extension/"]
#[include = "manifest.json"]
#[include = "background.js"]
#[include = "content.js"]
#[include = "lib/*.js"]
#[include = "icons/*.png"]
struct EmbeddedExtension;

/// Chromium-based browsers the host can be declared to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Browser {
    /// Brave (default).
    Brave,
    /// Chromium.
    Chromium,
    /// Google Chrome.
    Chrome,
    /// Microsoft Edge.
    Edge,
    /// Vivaldi.
    Vivaldi,
}

impl Browser {
    /// The browser's folder under the configuration directory.
    fn config_folder(self) -> &'static str {
        match self {
            Self::Brave => "BraveSoftware/Brave-Browser",
            Self::Chromium => "chromium",
            Self::Chrome => "google-chrome",
            Self::Edge => "microsoft-edge",
            Self::Vivaldi => "vivaldi",
        }
    }
}

/// Where the installation writes.
#[derive(Debug, Clone)]
pub struct Locations {
    /// `$XDG_DATA_HOME`, `~/.local/share` by default.
    pub data_dir: PathBuf,
    /// `$XDG_CONFIG_HOME`, `~/.config` by default.
    pub config_dir: PathBuf,
}

impl Locations {
    /// The user's directories, from the environment.
    ///
    /// # Errors
    ///
    /// Fails when neither the XDG variable nor `HOME` gives an absolute path.
    pub fn from_env() -> Result<Self, InstallError> {
        let home = absolute(std::env::var_os("HOME"));
        let directory = |variable: &str, under_home: &str| {
            absolute(std::env::var_os(variable))
                .or_else(|| home.as_ref().map(|home| home.join(under_home)))
                .ok_or(InstallError::NoHome)
        };
        Ok(Self {
            data_dir: directory("XDG_DATA_HOME", ".local/share")?,
            config_dir: directory("XDG_CONFIG_HOME", ".config")?,
        })
    }
}

/// An absolute path from an environment value (the XDG rules ignore others).
fn absolute(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

/// Why the installation failed.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    /// No absolute home, data or configuration directory.
    #[error("no home directory: set HOME")]
    NoHome,
    /// The binary has no absolute path, which the browser requires.
    #[error("the path of the hubergamof binary must be absolute: {0}")]
    RelativeBinary(PathBuf),
    /// The binary, or a folder above it, could be replaced by someone else:
    /// the browser would run whatever is there.
    #[error("{0} is writable by, or owned by, another account: install hubergamof elsewhere")]
    BinaryNotPrivate(PathBuf),
    /// The embedded manifest has no usable `key`.
    #[error("the extension's manifest has no valid key")]
    BadKey,
    /// A file could not be written.
    #[error("{path}: {source}")]
    Io {
        /// The file or folder concerned.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: io::Error,
    },
}

/// What was installed, for the user.
#[derive(Debug, PartialEq, Eq)]
pub struct Installed {
    /// Folder to "load unpacked" from.
    pub extension_dir: PathBuf,
    /// Manifest that lets the browser start the host.
    pub host_manifest: PathBuf,
    /// Identifier of the extension, which the host manifest allows.
    pub extension_id: String,
}

/// Identifier a browser derives from the public key of an extension: the
/// first 128 bits of its SHA-256, written with the letters `a` to `p`.
fn extension_id(key_base64: &str) -> Result<String, InstallError> {
    let der = BASE64
        .decode(key_base64)
        .map_err(|_| InstallError::BadKey)?;
    let digest = Sha256::digest(der);
    Ok(digest[..EXTENSION_ID_HASH_BYTES]
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .map(|nibble| char::from(b'a' + nibble))
        .collect())
}

/// The identifier of the embedded extension.
///
/// # Errors
///
/// Fails when the embedded manifest has no valid `key`.
pub fn embedded_extension_id() -> Result<String, InstallError> {
    let manifest = EmbeddedExtension::get("manifest.json").ok_or(InstallError::BadKey)?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest.data).map_err(|_| InstallError::BadKey)?;
    extension_id(manifest["key"].as_str().ok_or(InstallError::BadKey)?)
}

/// Sticky bit of a folder: others cannot replace what they do not own.
const STICKY: u32 = 0o1000;
/// Write bits of the group and of others.
const WRITABLE_BY_OTHERS: u32 = 0o022;

/// Refuses a `binary` the browser would run on trust if it, or a folder above
/// it, is owned by neither `user` nor root, or is writable by the group or
/// others (a sticky folder only protects what its user owns).
fn check_binary_is_private(binary: &Path, user: u32) -> Result<(), InstallError> {
    for path in binary.ancestors() {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_error(path)(error)),
        };
        let open_to_others =
            metadata.mode() & WRITABLE_BY_OTHERS != 0 && metadata.mode() & STICKY == 0;
        if open_to_others || (metadata.uid() != user && metadata.uid() != 0) {
            return Err(InstallError::BinaryNotPrivate(path.to_owned()));
        }
    }
    Ok(())
}

/// Whether `origin`, passed by the browser to the host, is the extension's.
///
/// # Errors
///
/// Fails when the embedded manifest has no valid `key`.
pub fn is_own_origin(origin: &str) -> Result<bool, InstallError> {
    Ok(origin == format!("{BROWSER_ORIGIN_PREFIX}{}/", embedded_extension_id()?))
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> InstallError + '_ {
    |source| InstallError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Replaces the extension folder with the embedded files.
fn write_extension(directory: &Path) -> Result<(), InstallError> {
    match std::fs::remove_dir_all(directory) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(io_error(directory)(error));
        }
        _ => {}
    }
    for name in EmbeddedExtension::iter() {
        let Some(file) = EmbeddedExtension::get(&name) else {
            continue;
        };
        let target = directory.join(name.as_ref());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io_error(parent))?;
        }
        std::fs::write(&target, file.data).map_err(io_error(&target))?;
    }
    Ok(())
}

/// Writes the extension and the native messaging manifest for `browser`.
///
/// # Errors
///
/// Fails when `binary` is not absolute or a file cannot be written.
pub fn install(
    locations: &Locations,
    binary: &Path,
    browser: Browser,
) -> Result<Installed, InstallError> {
    if !binary.is_absolute() {
        return Err(InstallError::RelativeBinary(binary.to_owned()));
    }
    let user = std::fs::metadata("/proc/self")
        .map_err(io_error(Path::new("/proc/self")))?
        .uid();
    check_binary_is_private(binary, user)?;
    let extension_id = embedded_extension_id()?;
    let extension_dir = locations.data_dir.join(EXTENSION_FOLDER);
    write_extension(&extension_dir)?;

    let hosts_dir = locations
        .config_dir
        .join(browser.config_folder())
        .join("NativeMessagingHosts");
    let host_manifest = hosts_dir.join(format!("{NATIVE_HOST_NAME}.json"));
    let contents = json!({
        "name": NATIVE_HOST_NAME,
        "description": "HuBergamof: relay between the Teams extension and the manager",
        "path": binary,
        "type": "stdio",
        "allowed_origins": [format!("{BROWSER_ORIGIN_PREFIX}{extension_id}/")],
    });
    std::fs::create_dir_all(&hosts_dir).map_err(io_error(&hosts_dir))?;
    std::fs::write(&host_manifest, format!("{contents:#}\n")).map_err(io_error(&host_manifest))?;
    Ok(Installed {
        extension_dir,
        host_manifest,
        extension_id,
    })
}

/// The command line the browser runs, as `hubergamof native-host`.
///
/// A browser starts a native messaging host with the extension's origin as
/// its only argument and cannot name a subcommand: that call is recognized
/// and given its command. Anything else is left as it is.
#[must_use]
pub fn with_browser_launch_command(args: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut args: Vec<String> = args.into_iter().collect();
    if args
        .get(1)
        .is_some_and(|first| first.starts_with(BROWSER_ORIGIN_PREFIX))
    {
        args.insert(1, "native-host".to_owned());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id of the key in `ui/teams-extension/manifest.json`; a new key
    /// changes it for everyone who installed the extension.
    const EXPECTED_ID: &str = "folmhggjjpejamnjijomnlckponidgcj";

    fn locations(root: &Path) -> Locations {
        Locations {
            data_dir: root.join("data"),
            config_dir: root.join("config"),
        }
    }

    #[test]
    fn the_embedded_key_gives_the_published_id() {
        assert_eq!(embedded_extension_id().unwrap(), EXPECTED_ID);
    }

    #[test]
    fn a_bad_key_has_no_id() {
        assert!(matches!(
            extension_id("not base64!"),
            Err(InstallError::BadKey)
        ));
    }

    #[test]
    fn install_writes_the_extension_and_a_host_manifest_that_allows_only_it() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("bin/hubergamof");
        let installed = install(&locations(root.path()), &binary, Browser::Brave).unwrap();

        for file in [
            "manifest.json",
            "background.js",
            "content.js",
            "lib/teams-dom.js",
        ] {
            assert!(installed.extension_dir.join(file).is_file(), "{file}");
        }
        assert!(!installed.extension_dir.join("test").exists());
        assert_eq!(
            installed.host_manifest,
            root.path()
                .join("config/BraveSoftware/Brave-Browser/NativeMessagingHosts")
                .join("com.hubergamof.teams.json")
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&installed.host_manifest).unwrap()).unwrap();
        assert_eq!(manifest["name"], NATIVE_HOST_NAME);
        assert_eq!(manifest["path"], binary.to_str().unwrap());
        assert_eq!(manifest["type"], "stdio");
        assert_eq!(
            manifest["allowed_origins"],
            json!([format!("chrome-extension://{EXPECTED_ID}/")])
        );
    }

    #[test]
    fn install_again_replaces_what_was_there() {
        let root = tempfile::tempdir().unwrap();
        let binary = Path::new("/usr/bin/hubergamof");
        let first = install(&locations(root.path()), binary, Browser::Chromium).unwrap();
        std::fs::write(first.extension_dir.join("stale.js"), "old").unwrap();
        let second = install(&locations(root.path()), binary, Browser::Chromium).unwrap();
        assert_eq!(first, second);
        assert!(!second.extension_dir.join("stale.js").exists());
    }

    #[test]
    fn a_binary_in_a_folder_open_to_others_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("bin");
        std::fs::create_dir(&folder).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o777)).unwrap();
        let outcome = install(
            &locations(root.path()),
            &folder.join("hubergamof"),
            Browser::Brave,
        );
        assert!(matches!(outcome, Err(InstallError::BinaryNotPrivate(path)) if path == folder));
        assert!(!root.path().join("config").exists());
    }

    #[test]
    fn only_the_embedded_origin_is_ours() {
        assert!(is_own_origin(&format!("chrome-extension://{EXPECTED_ID}/")).unwrap());
        assert!(!is_own_origin("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/").unwrap());
        assert!(!is_own_origin("").unwrap());
    }

    #[test]
    fn a_relative_binary_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let outcome = install(
            &locations(root.path()),
            Path::new("hubergamof"),
            Browser::Brave,
        );
        assert!(matches!(outcome, Err(InstallError::RelativeBinary(_))));
        assert!(!root.path().join("config").exists());
    }

    #[test]
    fn an_unwritable_destination_is_reported_with_its_path() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("config"), "a file, not a folder").unwrap();
        let outcome = install(
            &locations(root.path()),
            Path::new("/usr/bin/hubergamof"),
            Browser::Brave,
        );
        let Err(InstallError::Io { path, .. }) = outcome else {
            panic!("expected an I/O error");
        };
        assert!(path.starts_with(root.path().join("config")));
    }

    #[test]
    fn the_browser_call_gets_its_command() {
        let origin = format!("chrome-extension://{EXPECTED_ID}/");
        let args = |list: &[&str]| list.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            with_browser_launch_command(args(&["hubergamof", &origin])),
            args(&["hubergamof", "native-host", &origin])
        );
        assert_eq!(
            with_browser_launch_command(args(&["hubergamof", "open"])),
            args(&["hubergamof", "open"])
        );
        assert_eq!(
            with_browser_launch_command(args(&["hubergamof"])),
            args(&["hubergamof"])
        );
    }

    #[test]
    fn relative_xdg_values_are_ignored() {
        assert_eq!(absolute(Some("relative".into())), None);
        assert_eq!(absolute(None), None);
        assert_eq!(absolute(Some("/abs".into())), Some(PathBuf::from("/abs")));
    }
}
