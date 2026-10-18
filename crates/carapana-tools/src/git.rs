use std::{error::Error, fmt};

pub const MAX_GIT_STATUS_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_GIT_STATUS_ENTRIES: usize = 20_000;
pub const MAX_GIT_STATUS_PATH_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitStatusCode {
    Modified,
    TypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitStatusEntryKind {
    Changed,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
    Ignored,
}

/// Raw Git branch metadata. Values are untrusted bytes and must be escaped before display.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitBranchStatus {
    pub object_id: Option<Vec<u8>>,
    pub head: Option<Vec<u8>>,
    pub upstream: Option<Vec<u8>>,
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
}

/// One path from `git status --porcelain=v2 -z`.
///
/// Paths remain raw bytes: callers must not treat them as authorization or join them to a
/// workspace until a separate confinement check has succeeded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitStatusEntry {
    pub kind: GitStatusEntryKind,
    pub index_status: Option<GitStatusCode>,
    pub worktree_status: Option<GitStatusCode>,
    pub path: Vec<u8>,
    pub original_path: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitStatusSnapshot {
    pub branch: GitBranchStatus,
    pub entries: Vec<GitStatusEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitStatusParseError {
    OutputTooLarge,
    MissingTerminator,
    TooManyEntries,
    PathTooLarge,
    MalformedHeader,
    DuplicateHeader,
    InvalidAheadBehind,
    MalformedRecord,
    UnsupportedRecord,
}

impl fmt::Display for GitStatusParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::OutputTooLarge => "Git status output exceeds the 4 MiB limit",
            Self::MissingTerminator => "Git status output is missing its NUL terminator",
            Self::TooManyEntries => "Git status output exceeds the 20000-entry limit",
            Self::PathTooLarge => "Git status path exceeds the 16 KiB limit",
            Self::MalformedHeader => "Git status output contains a malformed branch header",
            Self::DuplicateHeader => "Git status output contains a duplicate branch header",
            Self::InvalidAheadBehind => "Git status output contains invalid ahead/behind counts",
            Self::MalformedRecord => "Git status output contains a malformed record",
            Self::UnsupportedRecord => "Git status output contains an unsupported record",
        };
        formatter.write_str(message)
    }
}

impl Error for GitStatusParseError {}

/// Parse bounded `git status --porcelain=v2 -z --branch` output without invoking Git or touching
/// the filesystem. NUL-delimited path bytes are preserved, including non-UTF-8 names.
pub fn parse_git_status_v2(output: &[u8]) -> Result<GitStatusSnapshot, GitStatusParseError> {
    if output.len() > MAX_GIT_STATUS_OUTPUT_BYTES {
        return Err(GitStatusParseError::OutputTooLarge);
    }
    if output.is_empty() {
        return Ok(GitStatusSnapshot::default());
    }
    if output.last() != Some(&0) {
        return Err(GitStatusParseError::MissingTerminator);
    }

    let mut snapshot = GitStatusSnapshot::default();
    let mut records = output[..output.len() - 1].split(|byte| *byte == 0);
    while let Some(record) = records.next() {
        if record.is_empty() {
            return Err(GitStatusParseError::MalformedRecord);
        }
        if record[0] == b'#' {
            parse_branch_header(record, &mut snapshot.branch)?;
            continue;
        }

        if record.starts_with(b"1 ") {
            let fields = split_fixed_fields(record, 9)?;
            let (index_status, worktree_status) = parse_xy(fields[1])?;
            push_entry(
                &mut snapshot,
                GitStatusEntry {
                    kind: GitStatusEntryKind::Changed,
                    index_status,
                    worktree_status,
                    path: copy_path(fields[8])?,
                    original_path: None,
                },
            )?;
            continue;
        }

        if record.starts_with(b"2 ") {
            let fields = split_fixed_fields(record, 10)?;
            let (index_status, worktree_status) = parse_xy(fields[1])?;
            let kind = parse_rename_score(fields[8])?;
            if kind == GitStatusEntryKind::Renamed
                && index_status != Some(GitStatusCode::Renamed)
                && worktree_status != Some(GitStatusCode::Renamed)
            {
                return Err(GitStatusParseError::MalformedRecord);
            }
            if kind == GitStatusEntryKind::Copied
                && index_status != Some(GitStatusCode::Copied)
                && worktree_status != Some(GitStatusCode::Copied)
            {
                return Err(GitStatusParseError::MalformedRecord);
            }
            let original_path = records.next().ok_or(GitStatusParseError::MalformedRecord)?;
            push_entry(
                &mut snapshot,
                GitStatusEntry {
                    kind,
                    index_status,
                    worktree_status,
                    path: copy_path(fields[9])?,
                    original_path: Some(copy_path(original_path)?),
                },
            )?;
            continue;
        }

        if record.starts_with(b"u ") {
            let fields = split_fixed_fields(record, 11)?;
            let (index_status, worktree_status) = parse_xy(fields[1])?;
            push_entry(
                &mut snapshot,
                GitStatusEntry {
                    kind: GitStatusEntryKind::Unmerged,
                    index_status,
                    worktree_status,
                    path: copy_path(fields[10])?,
                    original_path: None,
                },
            )?;
            continue;
        }

        if record.starts_with(b"? ") {
            push_entry(
                &mut snapshot,
                GitStatusEntry {
                    kind: GitStatusEntryKind::Untracked,
                    index_status: None,
                    worktree_status: None,
                    path: copy_path(&record[2..])?,
                    original_path: None,
                },
            )?;
            continue;
        }

        if record.starts_with(b"! ") {
            push_entry(
                &mut snapshot,
                GitStatusEntry {
                    kind: GitStatusEntryKind::Ignored,
                    index_status: None,
                    worktree_status: None,
                    path: copy_path(&record[2..])?,
                    original_path: None,
                },
            )?;
            continue;
        }

        return Err(GitStatusParseError::UnsupportedRecord);
    }

    Ok(snapshot)
}

fn parse_branch_header(
    record: &[u8],
    branch: &mut GitBranchStatus,
) -> Result<(), GitStatusParseError> {
    let rest = record
        .strip_prefix(b"# ")
        .ok_or(GitStatusParseError::MalformedHeader)?;
    let Some(separator) = rest.iter().position(|byte| *byte == b' ') else {
        if is_known_branch_header(rest) {
            return Err(GitStatusParseError::MalformedHeader);
        }
        return Ok(());
    };
    let key = &rest[..separator];
    let value = &rest[separator + 1..];

    match key {
        b"branch.oid" => set_branch_value(&mut branch.object_id, value),
        b"branch.head" => set_branch_value(&mut branch.head, value),
        b"branch.upstream" => set_branch_value(&mut branch.upstream, value),
        b"branch.ab" => {
            if branch.ahead.is_some() || branch.behind.is_some() {
                return Err(GitStatusParseError::DuplicateHeader);
            }
            let mut counts = value.split(|byte| *byte == b' ');
            let ahead = counts
                .next()
                .and_then(|count| parse_signed_count(count, b'+'))
                .ok_or(GitStatusParseError::InvalidAheadBehind)?;
            let behind = counts
                .next()
                .and_then(|count| parse_signed_count(count, b'-'))
                .ok_or(GitStatusParseError::InvalidAheadBehind)?;
            if counts.next().is_some() {
                return Err(GitStatusParseError::InvalidAheadBehind);
            }
            branch.ahead = Some(ahead);
            branch.behind = Some(behind);
            Ok(())
        }
        _ => Ok(()),
    }
}

fn is_known_branch_header(key: &[u8]) -> bool {
    matches!(
        key,
        b"branch.oid" | b"branch.head" | b"branch.upstream" | b"branch.ab"
    )
}

fn set_branch_value(slot: &mut Option<Vec<u8>>, value: &[u8]) -> Result<(), GitStatusParseError> {
    if value.is_empty() {
        return Err(GitStatusParseError::MalformedHeader);
    }
    if slot.is_some() {
        return Err(GitStatusParseError::DuplicateHeader);
    }
    *slot = Some(value.to_vec());
    Ok(())
}

fn parse_signed_count(value: &[u8], sign: u8) -> Option<usize> {
    let digits = value.strip_prefix(&[sign])?;
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

fn split_fixed_fields(
    record: &[u8],
    field_count: usize,
) -> Result<Vec<&[u8]>, GitStatusParseError> {
    let fields = record
        .splitn(field_count, |byte| *byte == b' ')
        .collect::<Vec<_>>();
    if fields.len() != field_count
        || fields[..field_count - 1]
            .iter()
            .any(|field| field.is_empty())
    {
        return Err(GitStatusParseError::MalformedRecord);
    }
    Ok(fields)
}

fn parse_xy(
    value: &[u8],
) -> Result<(Option<GitStatusCode>, Option<GitStatusCode>), GitStatusParseError> {
    if value.len() != 2 {
        return Err(GitStatusParseError::MalformedRecord);
    }
    Ok((parse_status_code(value[0])?, parse_status_code(value[1])?))
}

fn parse_status_code(value: u8) -> Result<Option<GitStatusCode>, GitStatusParseError> {
    let status = match value {
        b' ' | b'.' => return Ok(None),
        b'M' => GitStatusCode::Modified,
        b'T' => GitStatusCode::TypeChanged,
        b'A' => GitStatusCode::Added,
        b'D' => GitStatusCode::Deleted,
        b'R' => GitStatusCode::Renamed,
        b'C' => GitStatusCode::Copied,
        b'U' => GitStatusCode::Unmerged,
        _ => return Err(GitStatusParseError::MalformedRecord),
    };
    Ok(Some(status))
}

fn parse_rename_score(value: &[u8]) -> Result<GitStatusEntryKind, GitStatusParseError> {
    if value.len() < 2 || !value[1..].iter().all(u8::is_ascii_digit) {
        return Err(GitStatusParseError::MalformedRecord);
    }
    let _score = std::str::from_utf8(&value[1..])
        .ok()
        .and_then(|score| score.parse::<u8>().ok())
        .filter(|score| *score <= 100)
        .ok_or(GitStatusParseError::MalformedRecord)?;
    let kind = match value[0] {
        b'R' => GitStatusEntryKind::Renamed,
        b'C' => GitStatusEntryKind::Copied,
        _ => return Err(GitStatusParseError::MalformedRecord),
    };
    Ok(kind)
}

fn copy_path(path: &[u8]) -> Result<Vec<u8>, GitStatusParseError> {
    if path.is_empty() {
        return Err(GitStatusParseError::MalformedRecord);
    }
    if path.len() > MAX_GIT_STATUS_PATH_BYTES {
        return Err(GitStatusParseError::PathTooLarge);
    }
    Ok(path.to_vec())
}

fn push_entry(
    snapshot: &mut GitStatusSnapshot,
    entry: GitStatusEntry,
) -> Result<(), GitStatusParseError> {
    if snapshot.entries.len() >= MAX_GIT_STATUS_ENTRIES {
        return Err(GitStatusParseError::TooManyEntries);
    }
    snapshot.entries.push(entry);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        GitStatusCode, GitStatusEntryKind, GitStatusParseError, MAX_GIT_STATUS_ENTRIES,
        MAX_GIT_STATUS_OUTPUT_BYTES, MAX_GIT_STATUS_PATH_BYTES, parse_git_status_v2,
    };

    #[test]
    fn should_parse_branch_status_and_path_records() {
        let output = b"# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 abc123 def456 src/file with spaces.rs\0\
? new file.txt\0! ignored file.txt\0";

        let status = parse_git_status_v2(output).unwrap();

        assert_eq!(
            status.branch.object_id.as_deref(),
            Some(b"abc123".as_slice())
        );
        assert_eq!(status.branch.head.as_deref(), Some(b"main".as_slice()));
        assert_eq!(
            status.branch.upstream.as_deref(),
            Some(b"origin/main".as_slice())
        );
        assert_eq!(status.branch.ahead, Some(2));
        assert_eq!(status.branch.behind, Some(1));
        assert_eq!(status.entries.len(), 3);
        assert_eq!(status.entries[0].kind, GitStatusEntryKind::Changed);
        assert_eq!(status.entries[0].index_status, None);
        assert_eq!(
            status.entries[0].worktree_status,
            Some(GitStatusCode::Modified)
        );
        assert_eq!(status.entries[0].path, b"src/file with spaces.rs");
        assert_eq!(status.entries[1].kind, GitStatusEntryKind::Untracked);
        assert_eq!(status.entries[1].path, b"new file.txt");
        assert_eq!(status.entries[2].kind, GitStatusEntryKind::Ignored);
        assert_eq!(status.entries[2].path, b"ignored file.txt");
    }

    #[test]
    fn should_preserve_rename_paths_and_non_utf8_path_bytes() {
        let output = b"2 R. N... 100644 100644 100644 abc123 def456 R100 new name.rs\0old name.rs\0? bad-\xff-name\0\
2 C. N... 100644 100644 100644 abc123 def456 C75 copied name.rs\0source name.rs\0";

        let status = parse_git_status_v2(output).unwrap();

        assert_eq!(status.entries[0].kind, GitStatusEntryKind::Renamed);
        assert_eq!(status.entries[0].index_status, Some(GitStatusCode::Renamed));
        assert_eq!(status.entries[0].path, b"new name.rs");
        assert_eq!(
            status.entries[0].original_path.as_deref(),
            Some(b"old name.rs".as_slice())
        );
        assert_eq!(status.entries[1].path, b"bad-\xff-name");
        assert_eq!(status.entries[2].kind, GitStatusEntryKind::Copied);
        assert_eq!(status.entries[2].index_status, Some(GitStatusCode::Copied));
        assert_eq!(status.entries[2].path, b"copied name.rs");
        assert_eq!(
            status.entries[2].original_path.as_deref(),
            Some(b"source name.rs".as_slice())
        );
    }

    #[test]
    fn should_parse_unmerged_entries_without_interpreting_their_paths() {
        let output = b"u UU N... 100644 100644 100644 100644 abc123 def456 ghi789 conflict\tname\0";

        let status = parse_git_status_v2(output).unwrap();

        assert_eq!(status.entries.len(), 1);
        assert_eq!(status.entries[0].kind, GitStatusEntryKind::Unmerged);
        assert_eq!(
            status.entries[0].index_status,
            Some(GitStatusCode::Unmerged)
        );
        assert_eq!(
            status.entries[0].worktree_status,
            Some(GitStatusCode::Unmerged)
        );
        assert_eq!(status.entries[0].path, b"conflict\tname");
    }

    #[test]
    fn should_accept_empty_output_and_unknown_branch_headers() {
        assert!(parse_git_status_v2(b"").unwrap().entries.is_empty());
        let status = parse_git_status_v2(b"# branch.future value\0").unwrap();
        assert!(status.entries.is_empty());
    }

    #[test]
    fn should_reject_malformed_and_incomplete_records() {
        let cases = [
            (b"? file".as_slice(), GitStatusParseError::MissingTerminator),
            (
                b"x file\0".as_slice(),
                GitStatusParseError::UnsupportedRecord,
            ),
            (b"? \0".as_slice(), GitStatusParseError::MalformedRecord),
            (
                b"# branch.head main\0# branch.head next\0".as_slice(),
                GitStatusParseError::DuplicateHeader,
            ),
            (
                b"# branch.ab +1 nope\0".as_slice(),
                GitStatusParseError::InvalidAheadBehind,
            ),
            (
                b"2 R. N... 100644 100644 100644 abc def R100 new\0".as_slice(),
                GitStatusParseError::MalformedRecord,
            ),
            (
                b"1 .X N... 100644 100644 100644 abc def file\0".as_slice(),
                GitStatusParseError::MalformedRecord,
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(parse_git_status_v2(input), Err(expected));
        }
    }

    #[test]
    fn should_enforce_output_entry_and_path_limits() {
        let oversized_output = vec![b'x'; MAX_GIT_STATUS_OUTPUT_BYTES + 1];
        assert_eq!(
            parse_git_status_v2(&oversized_output),
            Err(GitStatusParseError::OutputTooLarge)
        );

        let mut too_many_entries = Vec::new();
        for _ in 0..=MAX_GIT_STATUS_ENTRIES {
            too_many_entries.extend_from_slice(b"? x\0");
        }
        assert_eq!(
            parse_git_status_v2(&too_many_entries),
            Err(GitStatusParseError::TooManyEntries)
        );

        let mut too_long_path = Vec::with_capacity(MAX_GIT_STATUS_PATH_BYTES + 3);
        too_long_path.extend_from_slice(b"? ");
        too_long_path.resize(MAX_GIT_STATUS_PATH_BYTES + 3, b'x');
        too_long_path.push(0);
        assert_eq!(
            parse_git_status_v2(&too_long_path),
            Err(GitStatusParseError::PathTooLarge)
        );
    }
}
