//! Bounded read-only workspace inspection primitives.
//!
//! These capabilities are intentionally not connected to a provider, daemon, CLI, or session
//! runner. Returned file contents remain untrusted and may contain secrets despite filename filters.

mod diff;
mod parser;

pub use diff::{
    MAX_DIFF_OUTPUT_BYTES, MAX_DIFF_SOURCE_BYTES, MAX_DIFF_TIME_MILLISECONDS, SnapshotDiffPreview,
    TextDiffPreview, TextDiffPreviewError, create_snapshot_diff_preview, create_text_diff_preview,
};
pub use parser::{
    MAX_PARSE_DIAGNOSTICS, MAX_PARSE_NODES, MAX_PARSE_TIME_MILLISECONDS, ParsedWorkspaceFile,
    WorkspaceCodeLanguage, WorkspaceParseError, WorkspaceSyntaxDiagnostic,
    WorkspaceSyntaxIssueKind, parse_workspace_file,
};

use std::{
    collections::HashMap,
    error::Error,
    fmt, io,
    path::{Component, Path, PathBuf},
};

use carapana_storage::{
    MAX_WORKSPACE_DIRECTORY_READ_ENTRIES, MAX_WORKSPACE_FILE_READ_BYTES,
    WorkspaceDirectoryEntryKind, WorkspaceDirectoryIdentity, WorkspaceDirectoryReadError,
    WorkspaceFileReadError, WorkspaceMetadata, WorkspaceMetadataError,
};
use ignore::gitignore::{Gitignore, GitignoreBuilder};

pub const MAX_WORKSPACE_WALK_DEPTH: usize = 64;
pub const MAX_WORKSPACE_WALK_ENTRIES: usize = MAX_WORKSPACE_DIRECTORY_READ_ENTRIES;
pub const MAX_WORKSPACE_LIST_FILES: usize = 10_000;
pub const MAX_WORKSPACE_SEARCH_FILES: usize = 512;
pub const MAX_WORKSPACE_SEARCH_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_WORKSPACE_SEARCH_MATCHES: usize = 200;
pub const MAX_WORKSPACE_SEARCH_QUERY_BYTES: usize = 512;

const MAX_IGNORE_RULE_FILES: usize = 512;
const MAX_IGNORE_RULE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SNIPPET_CONTEXT_BYTES: usize = 256;

#[derive(Clone, Debug, Default)]
struct IgnoreMatchers {
    custom: Option<Gitignore>,
    ignore: Option<Gitignore>,
    git: Option<Gitignore>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceFile {
    pub relative_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceFileListing {
    pub files: Vec<WorkspaceFile>,
    pub sensitive_entries_skipped: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceTraversalError {
    UnsupportedPlatform,
    WorkspaceChanged,
    WorkspaceUnavailable,
    TraversalFailed,
    InvalidEntryPath,
    SensitiveWorkspace,
    UnsafeIgnoreConfiguration,
    IgnoreConfigurationTooLarge,
}

impl fmt::Display for WorkspaceTraversalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnsupportedPlatform => "workspace traversal is unavailable on this platform",
            Self::WorkspaceChanged => "workspace identity changed during traversal",
            Self::WorkspaceUnavailable => "workspace is unavailable for traversal",
            Self::TraversalFailed => "workspace traversal failed",
            Self::InvalidEntryPath => "workspace traversal produced an invalid relative path",
            Self::SensitiveWorkspace => "refusing to traverse a workspace in a protected path",
            Self::UnsafeIgnoreConfiguration => {
                "refusing unsafe or invalid workspace ignore configuration"
            }
            Self::IgnoreConfigurationTooLarge => {
                "workspace ignore configuration exceeds the size limit"
            }
        };
        formatter.write_str(message)
    }
}

impl Error for WorkspaceTraversalError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceSearchRequest {
    pub query: String,
    pub case_sensitive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceSearchMatch {
    pub relative_path: PathBuf,
    pub line_number: usize,
    pub snippet: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceSearchResult {
    pub matches: Vec<WorkspaceSearchMatch>,
    pub files_scanned: usize,
    pub paths_skipped: usize,
    pub bytes_scanned: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceSearchError {
    EmptyQuery,
    QueryTooLong,
    QueryContainsControlCharacters,
    Traversal(WorkspaceTraversalError),
    WorkspaceChanged,
    WorkspaceUnavailable,
}

impl fmt::Display for WorkspaceSearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyQuery => "workspace search query must not be empty",
            Self::QueryTooLong => "workspace search query exceeds the 512-byte limit",
            Self::QueryContainsControlCharacters => {
                "workspace search query must not contain control characters"
            }
            Self::Traversal(error) => return write!(formatter, "{error}"),
            Self::WorkspaceChanged => "workspace identity changed during search",
            Self::WorkspaceUnavailable => "workspace became unavailable during search",
        };
        formatter.write_str(message)
    }
}

impl Error for WorkspaceSearchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Traversal(error) => Some(error),
            _ => None,
        }
    }
}

struct PendingDirectory {
    relative_path: PathBuf,
    identity: WorkspaceDirectoryIdentity,
}

/// List regular files from the explicitly selected workspace.
///
/// The walker honors `.gitignore`, `.ignore`, and `.carapanaignore`, disables global and parent
/// ignore configuration, does not descend through symlinks or cross filesystem boundaries, and
/// applies conservative default exclusions. Ignore files are read through the bounded no-follow
/// workspace reader and parsed from memory. Each directory is enumerated through storage's
/// descriptor-relative reader and tied to the identity captured when it was discovered. Output
/// paths are relative and sorted but remain untrusted data that callers must escape before rendering.
/// Any traversal error fails the operation rather than returning a possibly incomplete listing as
/// complete.
pub fn list_workspace_files(
    workspace: &WorkspaceMetadata,
) -> Result<WorkspaceFileListing, WorkspaceTraversalError> {
    verify_workspace(workspace).map_err(map_workspace_error)?;
    if workspace.is_sensitive_relative_path(Path::new("workspace-entry")) {
        return Err(WorkspaceTraversalError::SensitiveWorkspace);
    }
    let ignore_matchers = load_ignore_matchers(workspace)?;
    let root = workspace.path();
    let mut pending_directories = vec![PendingDirectory {
        relative_path: PathBuf::new(),
        identity: workspace.directory_identity(),
    }];
    let mut files = Vec::new();
    let mut sensitive_entries_skipped = 0_usize;
    let mut entries_seen = 1_usize;
    let mut truncated = false;

    while let Some(directory) = pending_directories.pop() {
        let remaining_entries = MAX_WORKSPACE_WALK_ENTRIES.saturating_sub(entries_seen);
        let mut listing = workspace
            .read_directory_with_limit(
                &directory.relative_path,
                &directory.identity,
                remaining_entries,
            )
            .map_err(map_directory_read_error)?;
        sensitive_entries_skipped =
            sensitive_entries_skipped.saturating_add(listing.sensitive_entries_skipped);
        if listing.truncated {
            truncated = true;
            break;
        }
        entries_seen = entries_seen.saturating_add(listing.entries_seen);
        listing
            .entries
            .sort_by(|left, right| left.name.cmp(&right.name));

        let mut stop_walk = false;
        for entry in listing.entries {
            let relative_path = directory.relative_path.join(&entry.name);
            if !is_valid_relative_path(&relative_path) {
                return Err(WorkspaceTraversalError::InvalidEntryPath);
            }
            if contains_default_excluded_directory(&relative_path) {
                continue;
            }
            if workspace.is_sensitive_relative_path(&relative_path) {
                sensitive_entries_skipped = sensitive_entries_skipped.saturating_add(1);
                continue;
            }

            let is_directory = entry.kind == WorkspaceDirectoryEntryKind::Directory;
            if is_ignored_path(&ignore_matchers, &root, &relative_path, is_directory) {
                continue;
            }

            match entry.kind {
                WorkspaceDirectoryEntryKind::Directory => {
                    let depth = relative_path.components().count();
                    if depth >= MAX_WORKSPACE_WALK_DEPTH {
                        truncated = true;
                        continue;
                    }
                    pending_directories.push(PendingDirectory {
                        relative_path,
                        identity: entry.identity().clone(),
                    });
                }
                WorkspaceDirectoryEntryKind::RegularFile => {
                    if files.len() == MAX_WORKSPACE_LIST_FILES {
                        truncated = true;
                        stop_walk = true;
                        break;
                    }
                    files.push(WorkspaceFile { relative_path });
                }
                WorkspaceDirectoryEntryKind::Symlink | WorkspaceDirectoryEntryKind::Other => {}
            }
        }

        if stop_walk {
            break;
        }
    }

    verify_workspace(workspace).map_err(map_workspace_error)?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    Ok(WorkspaceFileListing {
        files,
        sensitive_entries_skipped,
        truncated,
    })
}

/// Search regular text files using a literal query, with bounded traversal, reads, and output.
///
/// The query is case-sensitive by default. Case-insensitive matching folds ASCII letters only;
/// snippets replace terminal and bidirectional formatting controls, but file contents remain
/// untrusted data and may contain secrets not identified by the filename denylist.
pub fn search_workspace(
    workspace: &WorkspaceMetadata,
    request: &WorkspaceSearchRequest,
) -> Result<WorkspaceSearchResult, WorkspaceSearchError> {
    validate_search_query(&request.query)?;
    let listing = list_workspace_files(workspace).map_err(WorkspaceSearchError::Traversal)?;
    let query = if request.case_sensitive {
        request.query.clone()
    } else {
        request.query.to_ascii_lowercase()
    };

    let files_to_scan = listing.files.len().min(MAX_WORKSPACE_SEARCH_FILES);
    let mut matches = Vec::new();
    let mut files_scanned = 0_usize;
    let mut paths_skipped = listing
        .sensitive_entries_skipped
        .saturating_add(listing.files.len().saturating_sub(files_to_scan));
    let mut bytes_scanned = 0_usize;
    let mut truncated = listing.truncated || listing.files.len() > files_to_scan;

    for (file_index, file) in listing.files.iter().take(files_to_scan).enumerate() {
        let remaining_bytes = MAX_WORKSPACE_SEARCH_BYTES.saturating_sub(bytes_scanned);
        if remaining_bytes == 0 {
            paths_skipped = paths_skipped.saturating_add(files_to_scan - file_index);
            truncated = true;
            break;
        }
        let read_limit = remaining_bytes.min(MAX_WORKSPACE_FILE_READ_BYTES);
        let contents = match workspace.read_file_with_limit(&file.relative_path, read_limit) {
            Ok(contents) => contents,
            Err(WorkspaceFileReadError::Workspace(error)) => {
                return Err(map_workspace_error(error).into());
            }
            Err(WorkspaceFileReadError::FileTooLarge(_)) => {
                paths_skipped = paths_skipped.saturating_add(1);
                truncated = true;
                continue;
            }
            Err(_) => {
                paths_skipped = paths_skipped.saturating_add(1);
                continue;
            }
        };
        let next_total = bytes_scanned.saturating_add(contents.len());
        if next_total > MAX_WORKSPACE_SEARCH_BYTES {
            paths_skipped = paths_skipped.saturating_add(1);
            truncated = true;
            break;
        }
        bytes_scanned = next_total;
        if contents.contains(&0) {
            paths_skipped = paths_skipped.saturating_add(1);
            continue;
        }
        let Ok(text) = std::str::from_utf8(&contents) else {
            paths_skipped = paths_skipped.saturating_add(1);
            continue;
        };
        files_scanned = files_scanned.saturating_add(1);

        for (line_index, line) in text.lines().enumerate() {
            let folded_line = if request.case_sensitive {
                line.to_owned()
            } else {
                line.to_ascii_lowercase()
            };
            let Some(match_offset) = folded_line.find(&query) else {
                continue;
            };
            matches.push(WorkspaceSearchMatch {
                relative_path: file.relative_path.clone(),
                line_number: line_index.saturating_add(1),
                snippet: safe_snippet(line, match_offset, request.query.len()),
            });
            if matches.len() == MAX_WORKSPACE_SEARCH_MATCHES {
                truncated = true;
                break;
            }
        }
        if matches.len() == MAX_WORKSPACE_SEARCH_MATCHES {
            break;
        }
    }

    verify_workspace(workspace).map_err(map_workspace_error)?;

    Ok(WorkspaceSearchResult {
        matches,
        files_scanned,
        paths_skipped,
        bytes_scanned,
        truncated,
    })
}

fn validate_search_query(query: &str) -> Result<(), WorkspaceSearchError> {
    if query.trim().is_empty() {
        return Err(WorkspaceSearchError::EmptyQuery);
    }
    if query.len() > MAX_WORKSPACE_SEARCH_QUERY_BYTES {
        return Err(WorkspaceSearchError::QueryTooLong);
    }
    if query.chars().any(char::is_control) {
        return Err(WorkspaceSearchError::QueryContainsControlCharacters);
    }
    Ok(())
}

fn load_ignore_matchers(
    workspace: &WorkspaceMetadata,
) -> Result<HashMap<PathBuf, IgnoreMatchers>, WorkspaceTraversalError> {
    let root = workspace.path();
    let mut pending_directories = vec![PendingDirectory {
        relative_path: PathBuf::new(),
        identity: workspace.directory_identity(),
    }];
    let mut entries_seen = 1_usize;
    let mut ignore_files_seen = 0_usize;
    let mut ignore_bytes_seen = 0_u64;
    let mut matchers = HashMap::new();

    while let Some(directory) = pending_directories.pop() {
        let remaining_entries = MAX_WORKSPACE_WALK_ENTRIES.saturating_sub(entries_seen);
        let mut listing = workspace
            .read_directory_with_limit(
                &directory.relative_path,
                &directory.identity,
                remaining_entries,
            )
            .map_err(map_directory_read_error)?;
        if listing.truncated {
            return Err(WorkspaceTraversalError::TraversalFailed);
        }
        entries_seen = entries_seen.saturating_add(listing.entries_seen);
        listing
            .entries
            .sort_by(|left, right| left.name.cmp(&right.name));

        let relative_directory = directory.relative_path;
        let absolute_directory = root.join(&relative_directory);
        let mut directory_matchers = IgnoreMatchers::default();
        let ignore_files = [
            (".gitignore", IgnoreFileKind::Git),
            (".ignore", IgnoreFileKind::Ignore),
            (".carapanaignore", IgnoreFileKind::Custom),
        ];
        for (ignore_file_name, kind) in ignore_files {
            let Some(entry) = listing
                .entries
                .iter()
                .find(|entry| entry.name == std::ffi::OsStr::new(ignore_file_name))
            else {
                continue;
            };
            if entry.kind != WorkspaceDirectoryEntryKind::RegularFile {
                return Err(WorkspaceTraversalError::UnsafeIgnoreConfiguration);
            }
            let relative_path = relative_directory.join(ignore_file_name);
            let contents = match workspace.read_file(&relative_path) {
                Ok(contents) => contents,
                Err(WorkspaceFileReadError::Io(error))
                    if error.kind() == io::ErrorKind::NotFound =>
                {
                    continue;
                }
                Err(WorkspaceFileReadError::Workspace(error)) => {
                    return Err(map_workspace_error(error));
                }
                Err(WorkspaceFileReadError::Io(_)) => {
                    return Err(WorkspaceTraversalError::WorkspaceUnavailable);
                }
                Err(WorkspaceFileReadError::FileTooLarge(_)) => {
                    return Err(WorkspaceTraversalError::IgnoreConfigurationTooLarge);
                }
                Err(_) => return Err(WorkspaceTraversalError::UnsafeIgnoreConfiguration),
            };

            ignore_files_seen = ignore_files_seen.saturating_add(1);
            if ignore_files_seen > MAX_IGNORE_RULE_FILES {
                return Err(WorkspaceTraversalError::IgnoreConfigurationTooLarge);
            }
            if contents.len() > MAX_WORKSPACE_FILE_READ_BYTES {
                return Err(WorkspaceTraversalError::IgnoreConfigurationTooLarge);
            }
            let content_bytes = u64::try_from(contents.len()).unwrap_or(u64::MAX);
            ignore_bytes_seen = ignore_bytes_seen.saturating_add(content_bytes);
            if ignore_bytes_seen > MAX_IGNORE_RULE_BYTES as u64 {
                return Err(WorkspaceTraversalError::IgnoreConfigurationTooLarge);
            }

            let text = String::from_utf8(contents)
                .map_err(|_| WorkspaceTraversalError::UnsafeIgnoreConfiguration)?;
            let absolute_ignore_path = absolute_directory.join(ignore_file_name);
            let matcher = build_ignore_matcher(&absolute_directory, &absolute_ignore_path, &text)?;
            match kind {
                IgnoreFileKind::Git => directory_matchers.git = Some(matcher),
                IgnoreFileKind::Ignore => directory_matchers.ignore = Some(matcher),
                IgnoreFileKind::Custom => directory_matchers.custom = Some(matcher),
            }
        }
        if directory_matchers.custom.is_some()
            || directory_matchers.ignore.is_some()
            || directory_matchers.git.is_some()
        {
            matchers.insert(relative_directory.clone(), directory_matchers);
        }

        for entry in listing.entries {
            if entry.kind != WorkspaceDirectoryEntryKind::Directory {
                continue;
            }
            let relative_path = relative_directory.join(&entry.name);
            if contains_default_excluded_directory(&relative_path)
                || workspace.is_sensitive_relative_path(&relative_path)
            {
                continue;
            }
            let depth = relative_path.components().count();
            if depth >= MAX_WORKSPACE_WALK_DEPTH {
                continue;
            }
            pending_directories.push(PendingDirectory {
                relative_path,
                identity: entry.identity().clone(),
            });
        }
    }

    verify_workspace(workspace).map_err(map_workspace_error)?;
    Ok(matchers)
}

#[derive(Clone, Copy)]
enum IgnoreFileKind {
    Git,
    Ignore,
    Custom,
}

fn build_ignore_matcher(
    directory: &Path,
    ignore_file: &Path,
    contents: &str,
) -> Result<Gitignore, WorkspaceTraversalError> {
    let mut builder = GitignoreBuilder::new(directory);
    for (line_index, line) in contents.lines().enumerate() {
        let line = if line_index == 0 {
            line.trim_start_matches('\u{feff}')
        } else {
            line
        };
        builder
            .add_line(Some(ignore_file.to_owned()), line)
            .map_err(|_| WorkspaceTraversalError::UnsafeIgnoreConfiguration)?;
    }
    builder
        .build()
        .map_err(|_| WorkspaceTraversalError::UnsafeIgnoreConfiguration)
}

fn is_ignored_path(
    matchers: &HashMap<PathBuf, IgnoreMatchers>,
    workspace_root: &Path,
    relative_path: &Path,
    is_directory: bool,
) -> bool {
    let candidate = workspace_root.join(relative_path);
    let mut custom_match = None;
    let mut ignore_match = None;
    let mut git_match = None;
    let mut directory = relative_path.parent();

    while let Some(relative_directory) = directory {
        if let Some(directory_matchers) = matchers.get(relative_directory) {
            if custom_match.is_none() {
                custom_match = directory_matchers
                    .custom
                    .as_ref()
                    .map(|matcher| matcher.matched(&candidate, is_directory))
                    .filter(|matched| !matched.is_none())
                    .map(|matched| matched.is_ignore());
            }
            if ignore_match.is_none() {
                ignore_match = directory_matchers
                    .ignore
                    .as_ref()
                    .map(|matcher| matcher.matched(&candidate, is_directory))
                    .filter(|matched| !matched.is_none())
                    .map(|matched| matched.is_ignore());
            }
            if git_match.is_none() {
                git_match = directory_matchers
                    .git
                    .as_ref()
                    .map(|matcher| matcher.matched(&candidate, is_directory))
                    .filter(|matched| !matched.is_none())
                    .map(|matched| matched.is_ignore());
            }
            if custom_match.is_some() && ignore_match.is_some() && git_match.is_some() {
                break;
            }
        }
        if relative_directory.as_os_str().is_empty() {
            break;
        }
        directory = relative_directory.parent();
    }

    custom_match.or(ignore_match).or(git_match).unwrap_or(false)
}

fn contains_default_excluded_directory(relative_path: &Path) -> bool {
    for component in relative_path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        if matches!(
            name.to_str(),
            Some(".git" | "node_modules" | "target" | "dist" | "build" | ".cache" | "vendor")
        ) {
            return true;
        }
    }
    false
}

fn is_valid_relative_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return false;
    }
    let mut has_normal_component = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_normal_component = true,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return false;
            }
        }
    }
    has_normal_component
}

fn map_workspace_error(error: WorkspaceMetadataError) -> WorkspaceTraversalError {
    match error {
        WorkspaceMetadataError::UnsupportedPlatform => WorkspaceTraversalError::UnsupportedPlatform,
        WorkspaceMetadataError::IdentityChanged(_) => WorkspaceTraversalError::WorkspaceChanged,
        WorkspaceMetadataError::InvalidPath(_)
        | WorkspaceMetadataError::UnsafePath(_)
        | WorkspaceMetadataError::Io(_) => WorkspaceTraversalError::WorkspaceUnavailable,
    }
}

fn map_directory_read_error(error: WorkspaceDirectoryReadError) -> WorkspaceTraversalError {
    match error {
        WorkspaceDirectoryReadError::Workspace(error) => map_workspace_error(error),
        WorkspaceDirectoryReadError::InvalidRelativePath(_) => {
            WorkspaceTraversalError::InvalidEntryPath
        }
        WorkspaceDirectoryReadError::UnsafePath(_)
        | WorkspaceDirectoryReadError::NotDirectory(_) => WorkspaceTraversalError::TraversalFailed,
        WorkspaceDirectoryReadError::SensitivePath(_) => {
            WorkspaceTraversalError::SensitiveWorkspace
        }
        WorkspaceDirectoryReadError::Changed(_) => WorkspaceTraversalError::WorkspaceChanged,
        WorkspaceDirectoryReadError::Io(_) => WorkspaceTraversalError::WorkspaceUnavailable,
    }
}

impl From<WorkspaceTraversalError> for WorkspaceSearchError {
    fn from(error: WorkspaceTraversalError) -> Self {
        Self::Traversal(error)
    }
}

fn safe_snippet(line: &str, match_offset: usize, match_length: usize) -> String {
    let mut start = match_offset.saturating_sub(MAX_SNIPPET_CONTEXT_BYTES);
    while !line.is_char_boundary(start) {
        start = start.saturating_sub(1);
    }
    let proposed_end = match_offset
        .saturating_add(match_length)
        .saturating_add(MAX_SNIPPET_CONTEXT_BYTES)
        .min(line.len());
    let mut end = proposed_end;
    while !line.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }

    let selected = &line[start..end];
    let mut snippet = String::with_capacity(selected.len().saturating_add(6));
    if start > 0 {
        snippet.push('…');
    }
    for character in selected.chars() {
        if is_terminal_control(character) {
            snippet.push('\u{fffd}');
        } else {
            snippet.push(character);
        }
    }
    if end < line.len() {
        snippet.push('…');
    }
    snippet
}

fn is_terminal_control(character: char) -> bool {
    let code_point = character as u32;
    character.is_control()
        || matches!(
            code_point,
            0x061c | 0x200e..=0x200f | 0x202a..=0x202e | 0x2066..=0x2069
        )
}

fn verify_workspace(workspace: &WorkspaceMetadata) -> Result<(), WorkspaceMetadataError> {
    workspace.verify_current()
}

#[cfg(test)]
mod tests {
    use super::{
        WorkspaceSearchError, WorkspaceSearchRequest, list_workspace_files, search_workspace,
    };
    use carapana_storage::WorkspaceMetadata;
    use std::{
        collections::BTreeSet,
        fs,
        path::PathBuf,
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
            let path = std::fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!(
                    "carapana-tools-workspace-{}-{nonce}-{id}",
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

    #[cfg(unix)]
    #[test]
    fn should_list_relative_files_with_nested_ignore_rules_and_default_exclusions() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(
            workspace_path.join(".gitignore"),
            "ignored.log\nlayered.txt\nignored-directory/\n",
        )
        .unwrap();
        fs::write(workspace_path.join(".ignore"), "ignored.tmp\nlayered.txt\n").unwrap();
        fs::write(
            workspace_path.join(".carapanaignore"),
            "local-only.txt\n!layered.txt\n",
        )
        .unwrap();
        fs::create_dir_all(workspace_path.join("src")).unwrap();
        fs::write(workspace_path.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(workspace_path.join(".hidden-source.rs"), "hidden\n").unwrap();
        fs::write(workspace_path.join("ignored.log"), "ignored\n").unwrap();
        fs::write(workspace_path.join("ignored.tmp"), "ignored\n").unwrap();
        fs::write(workspace_path.join("local-only.txt"), "ignored\n").unwrap();
        fs::write(
            workspace_path.join("layered.txt"),
            "visible by custom allow rule\n",
        )
        .unwrap();
        fs::create_dir_all(workspace_path.join("ignored-directory")).unwrap();
        fs::write(
            workspace_path.join("ignored-directory/file.rs"),
            "ignored\n",
        )
        .unwrap();
        for excluded_directory in [
            ".git",
            "node_modules",
            "target",
            "dist",
            "build",
            ".cache",
            "vendor",
        ] {
            fs::create_dir_all(workspace_path.join(excluded_directory)).unwrap();
            fs::write(
                workspace_path.join(excluded_directory).join("excluded.rs"),
                "excluded\n",
            )
            .unwrap();
        }
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        let listing = list_workspace_files(&workspace).unwrap();
        let listed_paths: BTreeSet<_> = listing
            .files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();

        assert!(listed_paths.contains(&PathBuf::from("src/main.rs")));
        assert!(listed_paths.contains(&PathBuf::from(".hidden-source.rs")));
        assert!(listed_paths.contains(&PathBuf::from("layered.txt")));
        assert!(!listed_paths.contains(&PathBuf::from("ignored.log")));
        assert!(!listed_paths.contains(&PathBuf::from("ignored.tmp")));
        assert!(!listed_paths.contains(&PathBuf::from("local-only.txt")));
        assert!(!listed_paths.contains(&PathBuf::from("ignored-directory/file.rs")));
        for excluded_directory in [
            ".git",
            "node_modules",
            "target",
            "dist",
            "build",
            ".cache",
            "vendor",
        ] {
            assert!(
                !listed_paths
                    .iter()
                    .any(|path| path.starts_with(excluded_directory))
            );
        }
        assert!(
            listing
                .files
                .windows(2)
                .all(|pair| pair[0].relative_path <= pair[1].relative_path)
        );
        assert!(!listing.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn should_apply_nested_ignore_precedence_without_reopening_ignore_paths() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir_all(workspace_path.join("src")).unwrap();
        fs::write(
            workspace_path.join(".gitignore"),
            "shared.txt\nsrc/nested-allowed.txt\n",
        )
        .unwrap();
        fs::write(
            workspace_path.join(".ignore"),
            "shared.txt\nsrc/nested-allowed.txt\n",
        )
        .unwrap();
        fs::write(workspace_path.join(".carapanaignore"), "!shared.txt\n").unwrap();
        fs::write(
            workspace_path.join("src/.ignore"),
            "!nested-allowed.txt\nnested-ignored.txt\n",
        )
        .unwrap();
        fs::write(workspace_path.join("shared.txt"), "custom rules win\n").unwrap();
        fs::write(
            workspace_path.join("src/nested-allowed.txt"),
            "nested allow wins\n",
        )
        .unwrap();
        fs::write(
            workspace_path.join("src/nested-ignored.txt"),
            "nested ignore wins\n",
        )
        .unwrap();
        fs::write(
            workspace_path.join("src/unrelated.txt"),
            "ordinary source\n",
        )
        .unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        let listing = list_workspace_files(&workspace).unwrap();
        let listed_paths: BTreeSet<_> = listing
            .files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();

        assert!(listed_paths.contains(&PathBuf::from("shared.txt")));
        assert!(listed_paths.contains(&PathBuf::from("src/nested-allowed.txt")));
        assert!(listed_paths.contains(&PathBuf::from("src/unrelated.txt")));
        assert!(!listed_paths.contains(&PathBuf::from("src/nested-ignored.txt")));
    }

    #[cfg(unix)]
    #[test]
    fn should_reject_non_utf8_ignore_files() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(workspace_path.join(".ignore"), [0xff, b'\n']).unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        assert!(matches!(
            list_workspace_files(&workspace),
            Err(super::WorkspaceTraversalError::UnsafeIgnoreConfiguration)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_not_apply_ignore_rules_from_outside_the_workspace() {
        let directory = TestDirectory::new();
        let parent = directory.0.join("parent");
        let workspace_path = parent.join("workspace");
        fs::create_dir_all(workspace_path.join("src")).unwrap();
        fs::write(parent.join(".gitignore"), "workspace/src/\n").unwrap();
        fs::write(parent.join(".ignore"), "workspace/ignored.txt\n").unwrap();
        fs::write(workspace_path.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(workspace_path.join("ignored.txt"), "visible\n").unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        let listing = list_workspace_files(&workspace).unwrap();
        let listed_paths: BTreeSet<_> = listing
            .files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();

        assert!(listed_paths.contains(&PathBuf::from("src/main.rs")));
        assert!(listed_paths.contains(&PathBuf::from("ignored.txt")));
    }

    #[cfg(unix)]
    #[test]
    fn should_report_depth_truncation_only_when_directories_are_not_descended() {
        let directory = TestDirectory::new();
        let complete_workspace = directory.0.join("complete");
        let truncated_workspace = directory.0.join("truncated");
        fs::create_dir(&complete_workspace).unwrap();
        fs::create_dir(&truncated_workspace).unwrap();

        let mut complete_parent = complete_workspace.clone();
        for index in 0..super::MAX_WORKSPACE_WALK_DEPTH - 1 {
            complete_parent.push(format!("d{index:02}"));
            fs::create_dir(&complete_parent).unwrap();
        }
        fs::write(complete_parent.join("at-depth-limit.rs"), "visible\n").unwrap();

        let mut truncated_parent = truncated_workspace.clone();
        for index in 0..super::MAX_WORKSPACE_WALK_DEPTH {
            truncated_parent.push(format!("d{index:02}"));
            fs::create_dir(&truncated_parent).unwrap();
        }
        fs::create_dir(truncated_parent.join("too-deep")).unwrap();

        let complete =
            list_workspace_files(&WorkspaceMetadata::capture(&complete_workspace).unwrap())
                .unwrap();
        let truncated =
            list_workspace_files(&WorkspaceMetadata::capture(&truncated_workspace).unwrap())
                .unwrap();

        assert!(!complete.truncated);
        assert_eq!(complete.files.len(), 1);
        assert!(truncated.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn should_not_follow_symlinks_or_list_filename_protected_paths() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        let external_file = directory.0.join("external.txt");
        let external_directory = directory.0.join("external-directory");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(&external_file, "outside needle\n").unwrap();
        fs::create_dir(&external_directory).unwrap();
        fs::write(external_directory.join("outside.rs"), "outside source\n").unwrap();
        symlink(&external_file, workspace_path.join("linked-file.txt")).unwrap();
        symlink(&external_directory, workspace_path.join("linked-directory")).unwrap();
        fs::create_dir_all(workspace_path.join(".aws")).unwrap();
        fs::write(
            workspace_path.join(".aws/credentials"),
            "synthetic credential\n",
        )
        .unwrap();
        fs::create_dir_all(workspace_path.join("src")).unwrap();
        fs::write(workspace_path.join("src/id_ed25519"), "synthetic key\n").unwrap();
        fs::write(workspace_path.join(".env.example"), "TOKEN=placeholder\n").unwrap();
        fs::write(workspace_path.join("certificate.pem"), "synthetic key\n").unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        let listing = list_workspace_files(&workspace).unwrap();
        let listed_paths: BTreeSet<_> = listing
            .files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();

        assert!(!listed_paths.contains(&PathBuf::from("linked-file.txt")));
        assert!(!listed_paths.contains(&PathBuf::from("linked-directory/outside.rs")));
        assert!(!listed_paths.contains(&PathBuf::from(".aws/credentials")));
        assert!(!listed_paths.contains(&PathBuf::from("src/id_ed25519")));
        assert!(!listed_paths.contains(&PathBuf::from(".env.example")));
        assert!(!listed_paths.contains(&PathBuf::from("certificate.pem")));
        assert!(listing.sensitive_entries_skipped >= 4);
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_symlinked_ignore_configuration_without_reading_its_target() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        let external_ignore = directory.0.join("external-ignore");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(&external_ignore, "external-secret-pattern\n").unwrap();
        symlink(&external_ignore, workspace_path.join(".gitignore")).unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        assert!(matches!(
            list_workspace_files(&workspace),
            Err(super::WorkspaceTraversalError::UnsafeIgnoreConfiguration)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_oversized_ignore_configuration_before_walking_files() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(
            workspace_path.join(".ignore"),
            vec![b'x'; carapana_storage::MAX_WORKSPACE_FILE_READ_BYTES + 1],
        )
        .unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        assert!(matches!(
            list_workspace_files(&workspace),
            Err(super::WorkspaceTraversalError::IgnoreConfigurationTooLarge)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_fail_closed_when_the_captured_workspace_is_replaced() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        let moved_workspace = directory.0.join("moved-workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(workspace_path.join("source.rs"), "needle\n").unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();
        fs::rename(&workspace_path, &moved_workspace).unwrap();
        fs::create_dir(&workspace_path).unwrap();

        assert!(matches!(
            list_workspace_files(&workspace),
            Err(super::WorkspaceTraversalError::WorkspaceChanged)
        ));
        assert!(matches!(
            search_workspace(
                &workspace,
                &WorkspaceSearchRequest {
                    query: "needle".to_owned(),
                    case_sensitive: true,
                }
            ),
            Err(WorkspaceSearchError::Traversal(
                super::WorkspaceTraversalError::WorkspaceChanged
            ))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_search_only_listed_text_files_with_bounded_safe_snippets() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir_all(workspace_path.join("src")).unwrap();
        fs::write(
            workspace_path.join("src/main.rs"),
            "first line\nCafé ANSWER = 42;\n",
        )
        .unwrap();
        fs::write(
            workspace_path.join("src/terminal.rs"),
            "\u{1b}[31m\u{202e}needle\u{202c}\u{1b}[0m\n",
        )
        .unwrap();
        fs::write(workspace_path.join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(workspace_path.join("ignored.rs"), "needle\n").unwrap();
        fs::write(workspace_path.join("binary.bin"), b"needle\0not text").unwrap();
        fs::write(workspace_path.join("invalid-utf8.txt"), [0xff, 0xfe, b'n']).unwrap();
        fs::write(workspace_path.join(".env.example"), "needle\n").unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        let result = search_workspace(
            &workspace,
            &WorkspaceSearchRequest {
                query: "answer".to_owned(),
                case_sensitive: false,
            },
        )
        .unwrap();

        assert_eq!(result.matches.len(), 1);
        assert_eq!(
            result.matches[0].relative_path,
            PathBuf::from("src/main.rs")
        );
        assert_eq!(result.matches[0].line_number, 2);
        assert!(result.matches[0].snippet.contains("ANSWER = 42"));
        assert!(result.paths_skipped >= 3);

        let terminal_result = search_workspace(
            &workspace,
            &WorkspaceSearchRequest {
                query: "needle".to_owned(),
                case_sensitive: true,
            },
        )
        .unwrap();
        assert_eq!(
            terminal_result.matches.len(),
            1,
            "matched paths: {:?}",
            terminal_result
                .matches
                .iter()
                .map(|matched| &matched.relative_path)
                .collect::<Vec<_>>()
        );
        assert!(
            !terminal_result.matches[0]
                .snippet
                .chars()
                .any(super::is_terminal_control)
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_reject_invalid_search_queries_and_mark_bounded_results_as_truncated() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        let mut contents = String::new();
        for _ in 0..=super::MAX_WORKSPACE_SEARCH_MATCHES {
            contents.push_str("needle\n");
        }
        fs::write(workspace_path.join("matches.txt"), contents).unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        assert!(matches!(
            search_workspace(
                &workspace,
                &WorkspaceSearchRequest {
                    query: "  \t".to_owned(),
                    case_sensitive: true,
                }
            ),
            Err(WorkspaceSearchError::EmptyQuery)
        ));
        assert!(matches!(
            search_workspace(
                &workspace,
                &WorkspaceSearchRequest {
                    query: "needle\nline".to_owned(),
                    case_sensitive: true,
                }
            ),
            Err(WorkspaceSearchError::QueryContainsControlCharacters)
        ));
        assert!(matches!(
            search_workspace(
                &workspace,
                &WorkspaceSearchRequest {
                    query: "x".repeat(super::MAX_WORKSPACE_SEARCH_QUERY_BYTES + 1),
                    case_sensitive: true,
                }
            ),
            Err(WorkspaceSearchError::QueryTooLong)
        ));

        let result = search_workspace(
            &workspace,
            &WorkspaceSearchRequest {
                query: "needle".to_owned(),
                case_sensitive: true,
            },
        )
        .unwrap();
        assert_eq!(result.matches.len(), super::MAX_WORKSPACE_SEARCH_MATCHES);
        assert!(result.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn should_stop_search_at_the_total_read_budget() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        for index in 0..33 {
            let file_name = format!("file-{index:02}.txt");
            fs::write(
                workspace_path.join(file_name),
                vec![b'x'; carapana_storage::MAX_WORKSPACE_FILE_READ_BYTES],
            )
            .unwrap();
        }
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        let result = search_workspace(
            &workspace,
            &WorkspaceSearchRequest {
                query: "needle".to_owned(),
                case_sensitive: true,
            },
        )
        .unwrap();

        assert!(result.matches.is_empty());
        assert_eq!(result.files_scanned, 32);
        assert_eq!(result.bytes_scanned, super::MAX_WORKSPACE_SEARCH_BYTES);
        assert_eq!(result.paths_skipped, 1);
        assert!(result.truncated);
    }
}
