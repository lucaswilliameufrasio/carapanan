use std::{error::Error, fmt, path::Path, time::Duration};

use carapana_storage::WorkspaceFileSnapshot;
use similar::{ChangeTag, TextDiff};

pub const MAX_DIFF_SOURCE_BYTES: usize = 256 * 1024;
pub const MAX_DIFF_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_DIFF_TIME_MILLISECONDS: u64 = 250;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextDiffPreview {
    pub unified_diff: String,
    pub added_lines: usize,
    pub removed_lines: usize,
    pub truncated: bool,
}

/// A display-only diff whose source file identity is borrowed from its snapshot.
///
/// This value cannot outlive or be detached from the snapshot used to produce it.
pub struct SnapshotDiffPreview<'snapshot> {
    snapshot: &'snapshot WorkspaceFileSnapshot,
    preview: TextDiffPreview,
}

impl SnapshotDiffPreview<'_> {
    /// Returns an untrusted workspace path; callers must escape it before terminal/UI display.
    pub fn relative_path(&self) -> &Path {
        self.snapshot.relative_path()
    }

    pub fn preview(&self) -> &TextDiffPreview {
        &self.preview
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDiffPreviewError {
    SourceTooLarge,
    BinaryInput,
    InvalidUtf8Snapshot,
}

impl fmt::Display for TextDiffPreviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceTooLarge => {
                formatter.write_str("text diff input exceeds the 256 KiB per-source limit")
            }
            Self::BinaryInput => formatter.write_str("text diff does not accept NUL bytes"),
            Self::InvalidUtf8Snapshot => {
                formatter.write_str("workspace snapshot is not valid UTF-8 text")
            }
        }
    }
}

impl Error for TextDiffPreviewError {}

/// Create a bounded text diff whose baseline is captured by `snapshot`.
///
/// This reads no paths and performs no filesystem writes. The result borrows the snapshot so its
/// relative path cannot be substituted independently of the source bytes. It remains local,
/// display-only data and is not safe to share without separate content/secret checks.
pub fn create_snapshot_diff_preview<'snapshot>(
    snapshot: &'snapshot WorkspaceFileSnapshot,
    after: &str,
) -> Result<SnapshotDiffPreview<'snapshot>, TextDiffPreviewError> {
    let before = std::str::from_utf8(snapshot.contents())
        .map_err(|_| TextDiffPreviewError::InvalidUtf8Snapshot)?;
    let preview = create_text_diff_preview(before, after)?;

    Ok(SnapshotDiffPreview { snapshot, preview })
}

/// Create a bounded, display-only unified diff preview without reading or changing files.
///
/// Input lines are treated as untrusted text. The result sanitizes terminal and bidirectional
/// formatting controls; it is a preview, not a patch that can be applied losslessly. The diff
/// algorithm may approximate after its time budget and callers must escape the result for their UI.
pub fn create_text_diff_preview(
    before: &str,
    after: &str,
) -> Result<TextDiffPreview, TextDiffPreviewError> {
    if before.len() > MAX_DIFF_SOURCE_BYTES || after.len() > MAX_DIFF_SOURCE_BYTES {
        return Err(TextDiffPreviewError::SourceTooLarge);
    }
    if before.as_bytes().contains(&0) || after.as_bytes().contains(&0) {
        return Err(TextDiffPreviewError::BinaryInput);
    }

    let mut configuration = TextDiff::configure();
    configuration.timeout(Duration::from_millis(MAX_DIFF_TIME_MILLISECONDS));
    let diff = configuration.diff_lines(before, after);
    let mut added_lines = 0_usize;
    let mut removed_lines = 0_usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => {
                added_lines = added_lines.saturating_add(1);
            }
            ChangeTag::Delete => {
                removed_lines = removed_lines.saturating_add(1);
            }
            ChangeTag::Equal => {}
        }
    }

    let formatted_diff = diff
        .unified_diff()
        .context_radius(3)
        .header("before", "after")
        .to_string();
    let mut unified_diff = String::with_capacity(formatted_diff.len().min(MAX_DIFF_OUTPUT_BYTES));
    for character in formatted_diff.chars() {
        if character == '\n' || !is_terminal_control(character) {
            unified_diff.push(character);
        } else {
            unified_diff.push('\u{fffd}');
        }
    }

    let truncated = unified_diff.len() > MAX_DIFF_OUTPUT_BYTES;
    if truncated {
        let marker = "… diff preview truncated …\n";
        let mut end = MAX_DIFF_OUTPUT_BYTES.saturating_sub(marker.len());
        while !unified_diff.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        if let Some(last_line_end) = unified_diff[..end].rfind('\n') {
            end = last_line_end.saturating_add(1);
        }
        unified_diff.truncate(end);
        unified_diff.push_str(marker);
    }

    Ok(TextDiffPreview {
        unified_diff,
        added_lines,
        removed_lines,
        truncated,
    })
}

fn is_terminal_control(character: char) -> bool {
    let code_point = character as u32;
    character.is_control()
        || matches!(
            code_point,
            0x061c | 0x200e..=0x200f | 0x202a..=0x202e | 0x2066..=0x2069
        )
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_DIFF_SOURCE_BYTES, TextDiffPreviewError, create_snapshot_diff_preview,
        create_text_diff_preview, is_terminal_control,
    };
    use carapana_storage::WorkspaceMetadata;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::SystemTime,
    };

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    struct TestWorkspace {
        directory: PathBuf,
        workspace_path: PathBuf,
        metadata: WorkspaceMetadata,
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl TestWorkspace {
        fn new(filename: &str, contents: &[u8]) -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let directory = fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!("carapana-diff-{}-{nonce}-{id}", std::process::id()));
            fs::create_dir(&directory).unwrap();
            let workspace_path = directory.join("workspace");
            fs::create_dir(&workspace_path).unwrap();
            let file_path = workspace_path.join(filename);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(file_path, contents).unwrap();
            let metadata = WorkspaceMetadata::capture(&workspace_path).unwrap();

            Self {
                directory,
                workspace_path,
                metadata,
            }
        }

        fn snapshot(&self, filename: &str) -> carapana_storage::WorkspaceFileSnapshot {
            self.metadata.read_file_snapshot(filename).unwrap()
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn should_create_unified_preview_with_line_counts() {
        let preview =
            create_text_diff_preview("fn answer() {\n    41\n}\n", "fn answer() {\n    42\n}\n")
                .unwrap();

        assert_eq!(preview.added_lines, 1);
        assert_eq!(preview.removed_lines, 1);
        assert!(preview.unified_diff.contains("--- before\n"));
        assert!(preview.unified_diff.contains("+++ after\n"));
        assert!(preview.unified_diff.contains("-    41\n"));
        assert!(preview.unified_diff.contains("+    42\n"));
        assert!(!preview.truncated);
    }

    #[test]
    fn should_return_an_empty_preview_for_unchanged_text() {
        let preview = create_text_diff_preview("same\n", "same\n").unwrap();

        assert!(preview.unified_diff.is_empty());
        assert_eq!(preview.added_lines, 0);
        assert_eq!(preview.removed_lines, 0);
        assert!(!preview.truncated);
    }

    #[test]
    fn should_replace_terminal_and_bidi_controls_in_preview_lines() {
        let preview = create_text_diff_preview("before\n", "\u{1b}[31m\u{202e}after\n").unwrap();

        assert!(
            preview
                .unified_diff
                .chars()
                .all(|character| character == '\n' || !is_terminal_control(character))
        );
        assert!(preview.unified_diff.contains('\u{fffd}'));
    }

    #[test]
    fn should_reject_large_and_binary_inputs_before_diffing() {
        assert_eq!(
            create_text_diff_preview(&"x".repeat(MAX_DIFF_SOURCE_BYTES + 1), ""),
            Err(TextDiffPreviewError::SourceTooLarge)
        );
        assert_eq!(
            create_text_diff_preview("before\n", "after\0\n"),
            Err(TextDiffPreviewError::BinaryInput)
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn should_keep_snapshot_diff_bound_to_its_path_without_changing_the_file() {
        let workspace = TestWorkspace::new("src/main.rs", b"fn main() { 41 }\n");
        let snapshot = workspace.snapshot("src/main.rs");

        let diff = create_snapshot_diff_preview(&snapshot, "fn main() { 42 }\n").unwrap();

        assert_eq!(diff.relative_path(), Path::new("src/main.rs"));
        assert!(diff.preview().unified_diff.contains("-fn main() { 41 }"));
        assert!(diff.preview().unified_diff.contains("+fn main() { 42 }"));
        assert_eq!(
            fs::read(workspace.workspace_path.join("src/main.rs")).unwrap(),
            b"fn main() { 41 }\n"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn should_reject_non_utf8_snapshot_content_without_producing_a_diff() {
        let workspace = TestWorkspace::new("binary.dat", b"invalid UTF-8: \xff");
        let snapshot = workspace.snapshot("binary.dat");

        assert!(matches!(
            create_snapshot_diff_preview(&snapshot, "replacement\n"),
            Err(TextDiffPreviewError::InvalidUtf8Snapshot)
        ));
        assert_eq!(
            fs::read(workspace.workspace_path.join("binary.dat")).unwrap(),
            b"invalid UTF-8: \xff"
        );
    }
}
