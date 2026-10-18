use std::{error::Error, fmt};

use crate::git::MAX_GIT_STATUS_PATH_BYTES;

pub const MAX_GIT_WORKTREE_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_GIT_WORKTREES: usize = 256;
pub const MAX_GIT_WORKTREE_METADATA_BYTES: usize = 4 * 1024;

/// One entry from `git worktree list --porcelain -z`.
///
/// Paths, refs, and reasons are untrusted bytes. The parsed lock/prunable markers are Git metadata,
/// not proof that no other Carapanã session is editing the directory.
/// This type intentionally does not implement `Debug` to avoid accidental workspace-path logging.
#[derive(Clone, PartialEq, Eq)]
pub struct GitWorktreeMetadata {
    pub path: Vec<u8>,
    pub head: Vec<u8>,
    pub branch: Option<Vec<u8>>,
    pub detached: bool,
    pub bare: bool,
    pub locked: bool,
    pub lock_reason: Option<Vec<u8>>,
    pub prunable: bool,
    pub prune_reason: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitWorktreeParseError {
    OutputTooLarge,
    MissingTerminator,
    TooManyWorktrees,
    PathTooLarge,
    MetadataTooLarge,
    InvalidHead,
    MalformedRecord,
    DuplicateField,
}

impl fmt::Display for GitWorktreeParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::OutputTooLarge => "Git worktree output exceeds the 4 MiB limit",
            Self::MissingTerminator => "Git worktree output is missing its NUL terminator",
            Self::TooManyWorktrees => "Git worktree output exceeds the 256-entry limit",
            Self::PathTooLarge => "Git worktree path exceeds the 16 KiB limit",
            Self::MetadataTooLarge => "Git worktree metadata exceeds the 4 KiB limit",
            Self::InvalidHead => "Git worktree output contains an invalid HEAD object id",
            Self::MalformedRecord => "Git worktree output contains a malformed record",
            Self::DuplicateField => "Git worktree output contains a duplicate field",
        };
        formatter.write_str(message)
    }
}

impl Error for GitWorktreeParseError {}

/// Parse bounded `git worktree list --porcelain -z` output without invoking Git or accessing paths.
/// Unknown fields are ignored for forward compatibility; malformed known fields fail closed.
/// A future subprocess caller must also stop collecting stdout at the limit; this check cannot
/// bound memory already used to produce `output`.
pub fn parse_git_worktree_list_porcelain(
    output: &[u8],
) -> Result<Vec<GitWorktreeMetadata>, GitWorktreeParseError> {
    if output.len() > MAX_GIT_WORKTREE_OUTPUT_BYTES {
        return Err(GitWorktreeParseError::OutputTooLarge);
    }
    if output.is_empty() {
        return Ok(Vec::new());
    }
    if output.last() != Some(&0) {
        return Err(GitWorktreeParseError::MissingTerminator);
    }

    let mut worktrees = Vec::new();
    let mut current: Option<WorktreeBuilder> = None;
    for field in output[..output.len() - 1].split(|byte| *byte == 0) {
        if field.is_empty() {
            let worktree = current
                .take()
                .ok_or(GitWorktreeParseError::MalformedRecord)?;
            push_worktree(&mut worktrees, worktree.finish()?)?;
            continue;
        }

        if let Some(path) = field.strip_prefix(b"worktree ") {
            if current.is_some() {
                return Err(GitWorktreeParseError::MalformedRecord);
            }
            current = Some(WorktreeBuilder::new(path)?);
            continue;
        }

        let worktree = current
            .as_mut()
            .ok_or(GitWorktreeParseError::MalformedRecord)?;
        worktree.parse_field(field)?;
    }

    if let Some(worktree) = current {
        push_worktree(&mut worktrees, worktree.finish()?)?;
    }

    Ok(worktrees)
}

#[derive(Default)]
struct WorktreeBuilder {
    path: Option<Vec<u8>>,
    head: Option<Vec<u8>>,
    branch: Option<Vec<u8>>,
    detached: bool,
    bare: bool,
    locked: bool,
    lock_reason: Option<Vec<u8>>,
    prunable: bool,
    prune_reason: Option<Vec<u8>>,
}

impl WorktreeBuilder {
    fn new(path: &[u8]) -> Result<Self, GitWorktreeParseError> {
        if path.is_empty() {
            return Err(GitWorktreeParseError::MalformedRecord);
        }
        if path.len() > MAX_GIT_STATUS_PATH_BYTES {
            return Err(GitWorktreeParseError::PathTooLarge);
        }
        Ok(Self {
            path: Some(path.to_vec()),
            ..Self::default()
        })
    }

    fn parse_field(&mut self, field: &[u8]) -> Result<(), GitWorktreeParseError> {
        if let Some(head) = field.strip_prefix(b"HEAD ") {
            if self.head.is_some() {
                return Err(GitWorktreeParseError::DuplicateField);
            }
            if !is_valid_object_id(head) {
                return Err(GitWorktreeParseError::InvalidHead);
            }
            self.head = Some(head.to_vec());
            return Ok(());
        }

        if let Some(branch) = field.strip_prefix(b"branch ") {
            if self.branch.is_some() || branch.is_empty() || self.detached || self.bare {
                return Err(GitWorktreeParseError::MalformedRecord);
            }
            self.branch = Some(copy_metadata(branch)?);
            return Ok(());
        }

        if field == b"detached" {
            if self.detached || self.branch.is_some() || self.bare {
                return Err(GitWorktreeParseError::DuplicateField);
            }
            self.detached = true;
            return Ok(());
        }

        if field == b"bare" {
            if self.bare || self.branch.is_some() || self.detached {
                return Err(GitWorktreeParseError::DuplicateField);
            }
            self.bare = true;
            return Ok(());
        }

        if field == b"locked" {
            return self.set_locked(None);
        }
        if let Some(reason) = field.strip_prefix(b"locked ") {
            return self.set_locked(Some(reason));
        }

        if field == b"prunable" {
            return self.set_prunable(None);
        }
        if let Some(reason) = field.strip_prefix(b"prunable ") {
            return self.set_prunable(Some(reason));
        }

        if matches!(field, b"worktree" | b"HEAD" | b"branch") {
            return Err(GitWorktreeParseError::MalformedRecord);
        }

        Ok(())
    }

    fn set_locked(&mut self, reason: Option<&[u8]>) -> Result<(), GitWorktreeParseError> {
        if self.locked {
            return Err(GitWorktreeParseError::DuplicateField);
        }
        self.locked = true;
        if let Some(reason) = reason {
            self.lock_reason = Some(copy_metadata(reason)?);
        }
        Ok(())
    }

    fn set_prunable(&mut self, reason: Option<&[u8]>) -> Result<(), GitWorktreeParseError> {
        if self.prunable {
            return Err(GitWorktreeParseError::DuplicateField);
        }
        self.prunable = true;
        if let Some(reason) = reason {
            self.prune_reason = Some(copy_metadata(reason)?);
        }
        Ok(())
    }

    fn finish(self) -> Result<GitWorktreeMetadata, GitWorktreeParseError> {
        let path = self.path.ok_or(GitWorktreeParseError::MalformedRecord)?;
        let head = self.head.ok_or(GitWorktreeParseError::MalformedRecord)?;
        Ok(GitWorktreeMetadata {
            path,
            head,
            branch: self.branch,
            detached: self.detached,
            bare: self.bare,
            locked: self.locked,
            lock_reason: self.lock_reason,
            prunable: self.prunable,
            prune_reason: self.prune_reason,
        })
    }
}

fn is_valid_object_id(value: &[u8]) -> bool {
    matches!(value.len(), 40 | 64) && value.iter().all(u8::is_ascii_hexdigit)
}

fn copy_metadata(value: &[u8]) -> Result<Vec<u8>, GitWorktreeParseError> {
    if value.is_empty() {
        return Err(GitWorktreeParseError::MalformedRecord);
    }
    if value.len() > MAX_GIT_WORKTREE_METADATA_BYTES {
        return Err(GitWorktreeParseError::MetadataTooLarge);
    }
    Ok(value.to_vec())
}

fn push_worktree(
    worktrees: &mut Vec<GitWorktreeMetadata>,
    worktree: GitWorktreeMetadata,
) -> Result<(), GitWorktreeParseError> {
    if worktrees.len() >= MAX_GIT_WORKTREES {
        return Err(GitWorktreeParseError::TooManyWorktrees);
    }
    worktrees.push(worktree);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        GitWorktreeParseError, MAX_GIT_STATUS_PATH_BYTES, MAX_GIT_WORKTREE_METADATA_BYTES,
        MAX_GIT_WORKTREE_OUTPUT_BYTES, MAX_GIT_WORKTREES, parse_git_worktree_list_porcelain,
    };

    fn assert_error<T>(result: Result<T, GitWorktreeParseError>, expected: GitWorktreeParseError) {
        assert!(matches!(result, Err(actual) if actual == expected));
    }

    const HEAD_MAIN: &[u8] = b"0123456789012345678901234567890123456789";
    const HEAD_DETACHED: &[u8] = b"abcdefabcdefabcdefabcdefabcdefabcdefabcd";

    #[test]
    fn should_parse_branch_detached_and_status_markers() {
        let output = b"worktree /repos/main with spaces\0HEAD 0123456789012345678901234567890123456789\0branch refs/heads/main\0locked maintenance window\0\0\
worktree /repos/detached\ncheckout\0HEAD abcdefabcdefabcdefabcdefabcdefabcdefabcd\0detached\0prunable stale metadata\0";

        let worktrees = parse_git_worktree_list_porcelain(output).unwrap();

        assert_eq!(worktrees.len(), 2);
        assert_eq!(worktrees[0].path, b"/repos/main with spaces");
        assert_eq!(worktrees[0].head, HEAD_MAIN);
        assert_eq!(
            worktrees[0].branch.as_deref(),
            Some(b"refs/heads/main".as_slice())
        );
        assert!(worktrees[0].locked);
        assert_eq!(
            worktrees[0].lock_reason.as_deref(),
            Some(b"maintenance window".as_slice())
        );
        assert!(!worktrees[0].prunable);
        assert_eq!(worktrees[1].path, b"/repos/detached\ncheckout");
        assert_eq!(worktrees[1].head, HEAD_DETACHED);
        assert!(worktrees[1].detached);
        assert!(worktrees[1].prunable);
        assert_eq!(
            worktrees[1].prune_reason.as_deref(),
            Some(b"stale metadata".as_slice())
        );
    }

    #[test]
    fn should_preserve_non_utf8_paths_and_accept_unknown_fields() {
        let output = b"worktree /repos/bad-\xff-name\0HEAD 0123456789012345678901234567890123456789\0future-field value\0\0";

        let worktrees = parse_git_worktree_list_porcelain(output).unwrap();

        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].path, b"/repos/bad-\xff-name");
        assert!(!worktrees[0].bare);
        assert!(worktrees[0].branch.is_none());
    }

    #[test]
    fn should_parse_bare_worktrees_and_empty_output() {
        assert!(parse_git_worktree_list_porcelain(b"").unwrap().is_empty());

        let output = b"worktree /repos/bare\0HEAD 0123456789012345678901234567890123456789\0bare\0";
        let worktrees = parse_git_worktree_list_porcelain(output).unwrap();

        assert_eq!(worktrees.len(), 1);
        assert!(worktrees[0].bare);
        assert!(worktrees[0].branch.is_none());
        assert!(!worktrees[0].detached);
    }

    #[test]
    fn should_reject_malformed_duplicate_and_incomplete_fields() {
        let cases = [
            (b"worktree /repo".as_slice(), GitWorktreeParseError::MissingTerminator),
            (b"HEAD 0123456789012345678901234567890123456789\0".as_slice(), GitWorktreeParseError::MalformedRecord),
            (b"worktree /repo\0branch refs/heads/main\0\0".as_slice(), GitWorktreeParseError::MalformedRecord),
            (b"worktree /repo\0HEAD not-an-object-id\0\0".as_slice(), GitWorktreeParseError::InvalidHead),
            (b"worktree /repo\0HEAD 0123456789012345678901234567890123456789\0branch refs/heads/main\0branch refs/heads/other\0\0".as_slice(), GitWorktreeParseError::MalformedRecord),
            (b"worktree /repo\0HEAD 0123456789012345678901234567890123456789\0detached\0detached\0\0".as_slice(), GitWorktreeParseError::DuplicateField),
            (b"worktree /repo\0HEAD 0123456789012345678901234567890123456789\0locked\0locked reason\0\0".as_slice(), GitWorktreeParseError::DuplicateField),
        ];

        for (input, expected) in cases {
            assert_error(parse_git_worktree_list_porcelain(input), expected);
        }
    }

    #[test]
    fn should_enforce_worktree_path_metadata_and_collection_limits() {
        let mut too_long_path = Vec::with_capacity(MAX_GIT_STATUS_PATH_BYTES + 64);
        too_long_path.extend_from_slice(b"worktree ");
        too_long_path.resize(MAX_GIT_STATUS_PATH_BYTES + 10, b'x');
        too_long_path.extend_from_slice(b"\0HEAD 0123456789012345678901234567890123456789\0");
        assert_error(
            parse_git_worktree_list_porcelain(&too_long_path),
            GitWorktreeParseError::PathTooLarge,
        );

        let mut too_large_metadata =
            b"worktree /repo\0HEAD 0123456789012345678901234567890123456789\0branch ".to_vec();
        too_large_metadata.resize(
            too_large_metadata.len() + MAX_GIT_WORKTREE_METADATA_BYTES + 1,
            b'x',
        );
        too_large_metadata.push(0);
        assert_error(
            parse_git_worktree_list_porcelain(&too_large_metadata),
            GitWorktreeParseError::MetadataTooLarge,
        );

        let mut too_many_worktrees = Vec::new();
        for _ in 0..=MAX_GIT_WORKTREES {
            too_many_worktrees.extend_from_slice(
                b"worktree /repo\0HEAD 0123456789012345678901234567890123456789\0\0",
            );
        }
        assert_error(
            parse_git_worktree_list_porcelain(&too_many_worktrees),
            GitWorktreeParseError::TooManyWorktrees,
        );

        let oversized_output = vec![b'x'; MAX_GIT_WORKTREE_OUTPUT_BYTES + 1];
        assert_error(
            parse_git_worktree_list_porcelain(&oversized_output),
            GitWorktreeParseError::OutputTooLarge,
        );
    }
}
