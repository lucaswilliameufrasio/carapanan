use std::{
    error::Error,
    fmt,
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use crate::{Database, MigrationError};

const DATABASE_FILE: &str = "sessions.sqlite3";

#[derive(Debug)]
pub enum UserDatabaseError {
    MissingHomeDirectory,
    UnsupportedPlatform,
    UnsafePath(PathBuf),
    Io(io::Error),
    Migration(MigrationError),
}

impl fmt::Display for UserDatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHomeDirectory => {
                formatter.write_str("could not resolve the user's home directory")
            }
            Self::UnsupportedPlatform => {
                formatter.write_str("private user database is supported on Linux and macOS")
            }
            Self::UnsafePath(path) => write!(
                formatter,
                "refusing to use a non-private or unexpected database path: {}",
                path.display()
            ),
            Self::Io(error) => write!(
                formatter,
                "user database filesystem operation failed: {error}"
            ),
            Self::Migration(error) => {
                write!(formatter, "user database initialization failed: {error}")
            }
        }
    }
}

impl Error for UserDatabaseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Migration(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for UserDatabaseError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<MigrationError> for UserDatabaseError {
    fn from(error: MigrationError) -> Self {
        Self::Migration(error)
    }
}

/// Resolve the conventional per-user Carapanã data directory.
///
/// Linux honors an absolute `XDG_DATA_HOME`, otherwise using `$HOME/.local/share`.
/// macOS uses `$HOME/Library/Application Support`. Other platforms fail closed.
pub fn user_database_path() -> Result<PathBuf, UserDatabaseError> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(target_os = "linux")]
    let (platform, xdg_data_home) = (
        Platform::Linux,
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
    );
    #[cfg(target_os = "macos")]
    let (platform, xdg_data_home) = (Platform::MacOs, None);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let (platform, xdg_data_home) = (Platform::Unsupported, None);

    let directory = resolve_data_directory(platform, home, xdg_data_home)?;
    Ok(directory.join(DATABASE_FILE))
}

/// Open or initialize the private, per-user SQLite database.
pub fn open_user_database() -> Result<Database, UserDatabaseError> {
    open_database_at(&user_database_path()?)
}

fn open_database_at(path: &Path) -> Result<Database, UserDatabaseError> {
    let parent = path
        .parent()
        .ok_or_else(|| UserDatabaseError::UnsafePath(path.to_owned()))?;
    ensure_private_directory(parent)?;
    ensure_private_database_file(path)?;
    Ok(Database::open(path)?)
}

#[derive(Clone, Copy)]
enum Platform {
    #[cfg(any(target_os = "linux", test))]
    Linux,
    #[cfg(any(target_os = "macos", test))]
    MacOs,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Unsupported,
}

fn resolve_data_directory(
    platform: Platform,
    home: Option<PathBuf>,
    _xdg_data_home: Option<PathBuf>,
) -> Result<PathBuf, UserDatabaseError> {
    match platform {
        #[cfg(any(target_os = "linux", test))]
        Platform::Linux => {
            if let Some(xdg) = _xdg_data_home.filter(|path| path.is_absolute()) {
                return Ok(xdg.join("carapana"));
            }
            Ok(home_or_error(home)?.join(".local/share/carapana"))
        }
        #[cfg(any(target_os = "macos", test))]
        Platform::MacOs => Ok(home_or_error(home)?.join("Library/Application Support/Carapana")),
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        Platform::Unsupported => Err(UserDatabaseError::UnsupportedPlatform),
    }
}

fn home_or_error(home: Option<PathBuf>) -> Result<PathBuf, UserDatabaseError> {
    home.filter(|path| path.is_absolute())
        .ok_or(UserDatabaseError::MissingHomeDirectory)
}

#[cfg(unix)]
fn reject_symlink_components(path: &Path) -> Result<(), UserDatabaseError> {
    use std::path::Component;

    if !path.is_absolute() {
        return Err(UserDatabaseError::UnsafePath(path.to_owned()));
    }

    let mut component_path = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => component_path.push(component.as_os_str()),
            Component::Normal(name) => component_path.push(name),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(UserDatabaseError::UnsafePath(path.to_owned()));
            }
        }

        match fs::symlink_metadata(&component_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(UserDatabaseError::UnsafePath(component_path));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_private_directory(path: &Path) -> Result<(), UserDatabaseError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    reject_symlink_components(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.mode() & 0o077 != 0 {
                return Err(UserDatabaseError::UnsafePath(path.to_owned()));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| UserDatabaseError::UnsafePath(path.to_owned()))?;
            fs::create_dir_all(parent)?;
            reject_symlink_components(path)?;
            match fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            let metadata = fs::symlink_metadata(path)?;
            if !metadata.file_type().is_dir() || metadata.mode() & 0o077 != 0 {
                return Err(UserDatabaseError::UnsafePath(path.to_owned()));
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_directory(path: &Path) -> Result<(), UserDatabaseError> {
    let _ = path;
    Err(UserDatabaseError::UnsupportedPlatform)
}

#[cfg(unix)]
fn ensure_private_database_file(path: &Path) -> Result<(), UserDatabaseError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() || metadata.mode() & 0o077 != 0 {
                return Err(UserDatabaseError::UnsafePath(path.to_owned()));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
            {
                Ok(file) => drop(file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            let metadata = fs::symlink_metadata(path)?;
            if !metadata.file_type().is_file() || metadata.mode() & 0o077 != 0 {
                return Err(UserDatabaseError::UnsafePath(path.to_owned()));
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_database_file(path: &Path) -> Result<(), UserDatabaseError> {
    let _ = path;
    Err(UserDatabaseError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::{Platform, UserDatabaseError, open_database_at, resolve_data_directory};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::SystemTime,
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "carapana-user-db-{}-{nonce}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn should_resolve_linux_xdg_home_only_when_absolute() {
        let home = PathBuf::from("/home/operator");
        assert_eq!(
            resolve_data_directory(
                Platform::Linux,
                Some(home.clone()),
                Some(PathBuf::from("/mnt/user-data")),
            )
            .unwrap(),
            PathBuf::from("/mnt/user-data/carapana")
        );
        assert_eq!(
            resolve_data_directory(Platform::Linux, Some(home), Some(PathBuf::from("relative")),)
                .unwrap(),
            PathBuf::from("/home/operator/.local/share/carapana")
        );
    }

    #[test]
    fn should_resolve_macos_application_support_directory() {
        assert_eq!(
            resolve_data_directory(
                Platform::MacOs,
                Some(PathBuf::from("/Users/operator")),
                None,
            )
            .unwrap(),
            PathBuf::from("/Users/operator/Library/Application Support/Carapana")
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_create_private_database_and_refuse_preexisting_broad_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let root = TestDirectory::new();
        let path = root.0.join("private/sessions.sqlite3");
        let mut database = open_database_at(&path).unwrap();
        database.create_session("session-1", 1).unwrap();
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drop(database);
        assert!(open_database_at(&path).is_ok());

        let unsafe_path = root.0.join("unsafe/sessions.sqlite3");
        fs::create_dir_all(unsafe_path.parent().unwrap()).unwrap();
        fs::set_permissions(
            unsafe_path.parent().unwrap(),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert!(matches!(
            open_database_at(&unsafe_path),
            Err(UserDatabaseError::UnsafePath(_))
        ));

        let unsafe_file_dir = root.0.join("unsafe-file");
        fs::create_dir(&unsafe_file_dir).unwrap();
        fs::set_permissions(&unsafe_file_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let unsafe_file = unsafe_file_dir.join("sessions.sqlite3");
        fs::write(&unsafe_file, []).unwrap();
        fs::set_permissions(&unsafe_file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            open_database_at(&unsafe_file),
            Err(UserDatabaseError::UnsafePath(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_a_symlink_as_the_private_database_directory() {
        use std::os::unix::fs::symlink;

        let root = TestDirectory::new();
        let actual = root.0.join("actual");
        let linked = root.0.join("linked");
        fs::create_dir(&actual).unwrap();
        symlink(&actual, &linked).unwrap();
        assert!(matches!(
            open_database_at(&linked.join("sessions.sqlite3")),
            Err(UserDatabaseError::UnsafePath(_))
        ));

        let private_dir = root.0.join("private");
        fs::create_dir(&private_dir).unwrap();
        let database_target = root.0.join("external.sqlite3");
        fs::write(&database_target, []).unwrap();
        let database_link = private_dir.join("sessions.sqlite3");
        symlink(database_target, &database_link).unwrap();
        assert!(matches!(
            open_database_at(&database_link),
            Err(UserDatabaseError::UnsafePath(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_a_symlink_in_a_database_directory_ancestor() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = TestDirectory::new();
        let external = root.0.join("external");
        let linked_parent = root.0.join("linked-parent");
        fs::create_dir(&external).unwrap();
        fs::set_permissions(&external, fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&external, &linked_parent).unwrap();

        let database_path = linked_parent.join("private/sessions.sqlite3");
        assert!(matches!(
            open_database_at(&database_path),
            Err(UserDatabaseError::UnsafePath(_))
        ));
        assert!(
            fs::symlink_metadata(&linked_parent)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!external.join("private").exists());
        assert!(!external.join("private/sessions.sqlite3").exists());
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_relative_and_parent_traversal_database_paths() {
        let root = TestDirectory::new();
        let relative_path = Path::new("relative/sessions.sqlite3");
        assert!(matches!(
            open_database_at(relative_path),
            Err(UserDatabaseError::UnsafePath(_))
        ));

        let intermediate = root.0.join("intermediate");
        fs::create_dir(&intermediate).unwrap();
        let traversal_path = intermediate.join("../sessions.sqlite3");
        assert!(matches!(
            open_database_at(&traversal_path),
            Err(UserDatabaseError::UnsafePath(_))
        ));
        assert!(!root.0.join("sessions.sqlite3").exists());
    }
}
