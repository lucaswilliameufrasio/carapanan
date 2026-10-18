use crate::account::{ChatGptAccount, StoredAccount};
use carapana_storage::{UserDatabaseError, user_data_directory};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_CREDENTIAL_FILE_BYTES: u64 = 128 * 1024;
const HOST_ID_FILE: &str = "chatgpt-host-id";

pub struct CredentialStore {
    directory: PathBuf,
}

#[derive(Debug)]
pub enum CredentialStoreError {
    DirectoryUnavailable,
    UnsafePath,
    Io,
    InvalidCredentials,
    LockUnavailable,
    UnsupportedPlatform,
}

impl fmt::Display for CredentialStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectoryUnavailable => {
                formatter.write_str("private credential storage is unavailable")
            }
            Self::UnsafePath => {
                formatter.write_str("credential storage is not a private regular file or directory")
            }
            Self::Io => formatter.write_str("credential storage could not be accessed"),
            Self::InvalidCredentials => {
                formatter.write_str("saved ChatGPT credentials are invalid")
            }
            Self::LockUnavailable => {
                formatter.write_str("ChatGPT credentials are being updated by another process")
            }
            Self::UnsupportedPlatform => {
                formatter.write_str("credential storage requires Linux or macOS")
            }
        }
    }
}

impl Error for CredentialStoreError {}

impl From<UserDatabaseError> for CredentialStoreError {
    fn from(_: UserDatabaseError) -> Self {
        Self::DirectoryUnavailable
    }
}

impl CredentialStore {
    pub fn for_user() -> Result<Self, CredentialStoreError> {
        Self::at_directory(user_data_directory()?)
    }

    fn at_directory(directory: PathBuf) -> Result<Self, CredentialStoreError> {
        ensure_private_directory(&directory)?;
        Ok(Self { directory })
    }

    pub fn save(&self, account: &ChatGptAccount) -> Result<(), CredentialStoreError> {
        let record = account.to_stored();
        let expected_path = self.account_path(&record.client_id);
        let content =
            serde_json::to_vec(&record).map_err(|_| CredentialStoreError::InvalidCredentials)?;
        if content.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
            return Err(CredentialStoreError::InvalidCredentials);
        }
        self.write_atomic(&expected_path, &content)
    }

    pub fn load(&self, client_id: &str) -> Result<Option<ChatGptAccount>, CredentialStoreError> {
        let path = self.account_path(client_id);
        let Some(content) = self.read_private_file(&path)? else {
            return Ok(None);
        };
        let record = serde_json::from_slice::<StoredAccount>(&content)
            .map_err(|_| CredentialStoreError::InvalidCredentials)?;
        if record.client_id != client_id {
            return Err(CredentialStoreError::InvalidCredentials);
        }
        ChatGptAccount::from_stored(record)
            .map(Some)
            .ok_or(CredentialStoreError::InvalidCredentials)
    }

    pub fn accounts(&self) -> Result<Vec<ChatGptAccount>, CredentialStoreError> {
        ensure_private_directory(&self.directory)?;
        let entries = fs::read_dir(&self.directory).map_err(|_| CredentialStoreError::Io)?;
        let mut accounts = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|_| CredentialStoreError::Io)?;
            let path = entry.path();
            let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !filename.starts_with("chatgpt-") || path.extension().is_none_or(|ext| ext != "json")
            {
                continue;
            }
            let Some(content) = self.read_private_file(&path)? else {
                continue;
            };
            let record = serde_json::from_slice::<StoredAccount>(&content)
                .map_err(|_| CredentialStoreError::InvalidCredentials)?;
            if path != self.account_path(&record.client_id) {
                return Err(CredentialStoreError::InvalidCredentials);
            }
            let account = ChatGptAccount::from_stored(record)
                .ok_or(CredentialStoreError::InvalidCredentials)?;
            accounts.push(account);
        }
        accounts.sort_by(|left, right| left.client_id().cmp(right.client_id()));
        Ok(accounts)
    }

    pub fn remove(&self, client_id: &str) -> Result<bool, CredentialStoreError> {
        let _lock = self.lock_account(client_id)?;
        self.remove_locked(&_lock, client_id)
    }

    pub(crate) fn remove_locked(
        &self,
        _lock: &AccountLock,
        client_id: &str,
    ) -> Result<bool, CredentialStoreError> {
        ensure_private_directory(&self.directory)?;
        let path = self.account_path(client_id);
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(CredentialStoreError::Io),
        }
    }

    pub fn host_id(&self) -> Result<String, CredentialStoreError> {
        ensure_private_directory(&self.directory)?;
        let path = self.directory.join(HOST_ID_FILE);
        if let Some(content) = self.read_private_file(&path)? {
            let value =
                String::from_utf8(content).map_err(|_| CredentialStoreError::InvalidCredentials)?;
            let id = value.trim();
            if let Some(uuid) = id.strip_prefix("urn:uuid:")
                && Uuid::parse_str(uuid).is_ok_and(|parsed| parsed.get_version_num() == 4)
            {
                return Ok(id.to_owned());
            }
            return Err(CredentialStoreError::InvalidCredentials);
        }

        let id = format!("urn:uuid:{}", Uuid::new_v4());
        match self.write_new_private_file(&path, id.as_bytes()) {
            Ok(()) => Ok(id),
            Err(CredentialStoreError::Io) if path.exists() => self.host_id(),
            Err(error) => Err(error),
        }
    }

    fn account_path(&self, client_id: &str) -> PathBuf {
        let digest = Sha256::digest(client_id.as_bytes());
        let mut filename = String::with_capacity(64);
        for byte in digest {
            use fmt::Write as _;
            let _ = write!(filename, "{byte:02x}");
        }
        self.directory.join(format!("chatgpt-{filename}.json"))
    }

    fn read_private_file(&self, path: &Path) -> Result<Option<Vec<u8>>, CredentialStoreError> {
        ensure_private_directory(&self.directory)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = match options.open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
                return Err(CredentialStoreError::UnsafePath);
            }
            Err(_) => return Err(CredentialStoreError::Io),
        };
        validate_private_file(&file)?;
        let mut content = Vec::new();
        file.take(MAX_CREDENTIAL_FILE_BYTES + 1)
            .read_to_end(&mut content)
            .map_err(|_| CredentialStoreError::Io)?;
        if content.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
            return Err(CredentialStoreError::InvalidCredentials);
        }
        Ok(Some(content))
    }

    fn write_atomic(&self, path: &Path, content: &[u8]) -> Result<(), CredentialStoreError> {
        ensure_private_directory(&self.directory)?;
        let temporary = self
            .directory
            .join(format!(".chatgpt-{}.tmp", Uuid::new_v4()));
        self.write_new_private_file(&temporary, content)?;
        let result = fs::rename(&temporary, path).map_err(|_| CredentialStoreError::Io);
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            return result;
        }
        File::open(&self.directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| CredentialStoreError::Io)
    }

    fn write_new_private_file(
        &self,
        path: &Path,
        content: &[u8],
    ) -> Result<(), CredentialStoreError> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|_| CredentialStoreError::Io)?;
        file.write_all(content)
            .map_err(|_| CredentialStoreError::Io)?;
        file.sync_all().map_err(|_| CredentialStoreError::Io)
    }

    pub(crate) fn lock_account(
        &self,
        client_id: &str,
    ) -> Result<AccountLock, CredentialStoreError> {
        ensure_private_directory(&self.directory)?;
        let path = self.account_path(client_id).with_extension("lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(path).map_err(|_| CredentialStoreError::Io)?;
        validate_private_file(&file)?;
        file.lock_exclusive()
            .map_err(|_| CredentialStoreError::LockUnavailable)?;
        Ok(AccountLock { _file: file })
    }
}

pub(crate) struct AccountLock {
    _file: File,
}

#[cfg(unix)]
fn ensure_private_directory(path: &Path) -> Result<(), CredentialStoreError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.mode() & 0o077 != 0 {
                return Err(CredentialStoreError::UnsafePath);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(path).map_err(|_| CredentialStoreError::Io)?;
            let metadata = fs::symlink_metadata(path).map_err(|_| CredentialStoreError::Io)?;
            if !metadata.file_type().is_dir() || metadata.mode() & 0o077 != 0 {
                return Err(CredentialStoreError::UnsafePath);
            }
        }
        Err(_) => return Err(CredentialStoreError::Io),
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_directory(_: &Path) -> Result<(), CredentialStoreError> {
    Err(CredentialStoreError::UnsupportedPlatform)
}

#[cfg(unix)]
fn validate_private_file(file: &File) -> Result<(), CredentialStoreError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata().map_err(|_| CredentialStoreError::Io)?;
    if !metadata.file_type().is_file() || metadata.mode() & 0o077 != 0 {
        return Err(CredentialStoreError::UnsafePath);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_file(_: &File) -> Result<(), CredentialStoreError> {
    Err(CredentialStoreError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::{CredentialStore, CredentialStoreError};
    use crate::{ChatGptAccount, account::ChatGptAccountData};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::SystemTime,
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!(
                    "carapana-openai-{}-{timestamp}-{id}",
                    std::process::id()
                ));
            fs::create_dir(&path).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn account(scopes: Vec<String>) -> ChatGptAccount {
        ChatGptAccount::new(ChatGptAccountData {
            client_id: "oaiapp_test-registration".to_owned(),
            subject: "subject-test".to_owned(),
            email: Some("operator@example.invalid".to_owned()),
            scopes,
            access_token: "test-access-token".to_owned(),
            refresh_token: Some("test-refresh-token".to_owned()),
            id_token: "test-id-token".to_owned(),
            expires_at_unix_seconds: 4_000_000_000,
        })
    }

    #[cfg(unix)]
    #[test]
    fn credentials_round_trip_in_owner_only_files_and_host_id_is_stable() {
        use std::os::unix::fs::PermissionsExt;

        let directory = TestDirectory::new();
        let store = CredentialStore::at_directory(directory.0.clone()).unwrap();
        let account = account(vec!["chatgpt.tokens.use.direct".to_owned()]);
        let expected_summary = account.summary();

        store.save(&account).unwrap();
        let loaded = store.load(account.client_id()).unwrap().unwrap();
        assert_eq!(loaded.summary(), expected_summary);
        assert!(loaded.plan_usage_enabled());

        let credential_path = store.account_path(account.client_id());
        assert_eq!(
            fs::metadata(credential_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let first_host_id = store.host_id().unwrap();
        let second_store = CredentialStore::at_directory(directory.0.clone()).unwrap();
        assert_eq!(second_store.host_id().unwrap(), first_host_id);
        assert!(first_host_id.starts_with("urn:uuid:"));
    }

    #[cfg(unix)]
    #[test]
    fn credentials_without_plan_scope_cannot_enable_plan_usage() {
        let directory = TestDirectory::new();
        let store = CredentialStore::at_directory(directory.0.clone()).unwrap();
        let account = account(vec!["openid".to_owned(), "profile".to_owned()]);

        store.save(&account).unwrap();
        let loaded = store.load(account.client_id()).unwrap().unwrap();

        assert!(!loaded.plan_usage_enabled());
        assert!(!loaded.summary().plan_usage_enabled);
    }

    #[cfg(unix)]
    #[test]
    fn terminal_refresh_rejection_can_clear_only_the_refresh_token_and_keep_account_mapping() {
        let directory = TestDirectory::new();
        let store = CredentialStore::at_directory(directory.0.clone()).unwrap();
        let account = account(vec!["chatgpt.tokens.use.direct".to_owned()]);
        let client_id = account.client_id().to_owned();

        store.save(&account).unwrap();
        store.save(&account.without_refresh_token()).unwrap();
        let loaded = store.load(&client_id).unwrap().unwrap();
        let listed = store.accounts().unwrap();

        assert_eq!(loaded.client_id(), client_id);
        assert_eq!(loaded.subject(), "subject-test");
        assert_eq!(loaded.refresh_token(), None);
        assert!(!loaded.summary().has_refresh_token);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].client_id(), client_id);
    }

    #[cfg(unix)]
    #[test]
    fn credential_store_rejects_a_symlink_instead_of_following_it() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let store = CredentialStore::at_directory(directory.0.clone()).unwrap();
        let client_id = "oaiapp_test-registration";
        let path = store.account_path(client_id);
        let target = directory.0.join("external.json");
        fs::write(&target, b"{}").unwrap();
        symlink(&target, &path).unwrap();

        assert!(matches!(
            store.load(client_id),
            Err(CredentialStoreError::Io | CredentialStoreError::UnsafePath)
        ));
        assert_eq!(fs::read(target).unwrap(), b"{}");
    }

    #[cfg(unix)]
    #[test]
    fn credential_store_rejects_a_directory_with_broad_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = TestDirectory::new();
        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(matches!(
            CredentialStore::at_directory(directory.0.clone()),
            Err(CredentialStoreError::UnsafePath)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn account_lock_serializes_token_refreshes_across_store_instances() {
        let directory = TestDirectory::new();
        let first_store = CredentialStore::at_directory(directory.0.clone()).unwrap();
        let second_store = CredentialStore::at_directory(directory.0.clone()).unwrap();
        let client_id = "oaiapp_test-registration";
        let first_lock = first_store.lock_account(client_id).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let second_lock = second_store.lock_account(client_id).unwrap();
            sender.send(()).unwrap();
            drop(second_lock);
        });

        assert!(
            receiver
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err()
        );
        drop(first_lock);
        receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        worker.join().unwrap();
    }
}
