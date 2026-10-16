use std::{
    error::Error,
    fmt,
    ops::ControlFlow,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use carapana_storage::{WorkspaceFileReadError, WorkspaceMetadata};
use tree_sitter::{Language, Node, ParseOptions, ParseState, Parser};

pub const MAX_PARSE_DIAGNOSTICS: usize = 100;
pub const MAX_PARSE_NODES: usize = 100_000;
pub const MAX_PARSE_TIME_MILLISECONDS: u64 = 250;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceCodeLanguage {
    Rust,
    TypeScript,
    Tsx,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceSyntaxIssueKind {
    Error,
    MissingNode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceSyntaxDiagnostic {
    pub kind: WorkspaceSyntaxIssueKind,
    pub node_kind: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub start_column_bytes: usize,
    pub end_line: usize,
    pub end_column_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedWorkspaceFile {
    pub relative_path: PathBuf,
    pub language: WorkspaceCodeLanguage,
    pub root_node_kind: String,
    pub has_syntax_errors: bool,
    pub syntax_error_count: usize,
    pub nodes_visited: usize,
    pub diagnostics: Vec<WorkspaceSyntaxDiagnostic>,
    pub diagnostics_truncated: bool,
    pub node_walk_truncated: bool,
}

#[derive(Debug)]
pub enum WorkspaceParseError {
    UnsupportedLanguage,
    InvalidUtf8,
    IncompatibleGrammar,
    ParseTimedOut,
    ParseCancelled,
    WorkspaceRead(WorkspaceFileReadError),
}

impl fmt::Display for WorkspaceParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedLanguage => {
                formatter.write_str("workspace file extension has no enabled parser")
            }
            Self::InvalidUtf8 => formatter.write_str("workspace source file is not valid UTF-8"),
            Self::IncompatibleGrammar => {
                formatter.write_str("Tree-sitter grammar is incompatible with the parser runtime")
            }
            Self::ParseTimedOut => {
                formatter.write_str("Tree-sitter parsing exceeded its time limit")
            }
            Self::ParseCancelled => formatter.write_str("Tree-sitter parsing was cancelled"),
            Self::WorkspaceRead(error) => {
                write!(formatter, "workspace source read failed: {error}")
            }
        }
    }
}

impl Error for WorkspaceParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WorkspaceRead(error) => Some(error),
            _ => None,
        }
    }
}

/// Parse one explicitly selected Rust, TypeScript, or TSX file on demand.
///
/// Source is read through the workspace's no-follow reader and is never copied into the result.
/// Returned node kinds, paths, and byte positions are untrusted metadata; syntax errors are normal
/// parse results and do not mean the file was compiled or validated semantically.
pub fn parse_workspace_file(
    workspace: &WorkspaceMetadata,
    relative_path: impl AsRef<Path>,
) -> Result<ParsedWorkspaceFile, WorkspaceParseError> {
    let relative_path = relative_path.as_ref();
    let language =
        language_for_path(relative_path).ok_or(WorkspaceParseError::UnsupportedLanguage)?;
    let source = workspace
        .read_file(relative_path)
        .map_err(WorkspaceParseError::WorkspaceRead)?;
    let source = std::str::from_utf8(&source).map_err(|_| WorkspaceParseError::InvalidUtf8)?;

    let language_definition: Language = match language {
        WorkspaceCodeLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
        WorkspaceCodeLanguage::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        WorkspaceCodeLanguage::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    };
    let mut parser = Parser::new();
    parser
        .set_language(&language_definition)
        .map_err(|_| WorkspaceParseError::IncompatibleGrammar)?;

    let deadline = Instant::now() + Duration::from_millis(MAX_PARSE_TIME_MILLISECONDS);
    let mut timed_out = false;
    let mut progress_callback = |_state: &ParseState| {
        if Instant::now() >= deadline {
            timed_out = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let parse_options = ParseOptions::new().progress_callback(&mut progress_callback);
    let source_bytes = source.as_bytes();
    let mut input_callback = |byte_index: usize, _position: tree_sitter::Point| {
        let byte_index = byte_index.min(source_bytes.len());
        &source_bytes[byte_index..]
    };
    let Some(tree) = parser.parse_with_options(&mut input_callback, None, Some(parse_options))
    else {
        return if timed_out {
            Err(WorkspaceParseError::ParseTimedOut)
        } else {
            Err(WorkspaceParseError::ParseCancelled)
        };
    };

    let root = tree.root_node();
    let collected = collect_syntax_diagnostics(root, MAX_PARSE_NODES, MAX_PARSE_DIAGNOSTICS);

    Ok(ParsedWorkspaceFile {
        relative_path: relative_path.to_owned(),
        language,
        root_node_kind: root.kind().to_owned(),
        has_syntax_errors: root.has_error(),
        syntax_error_count: collected.syntax_error_count,
        nodes_visited: collected.nodes_visited,
        diagnostics: collected.diagnostics,
        diagnostics_truncated: collected.diagnostics_truncated,
        node_walk_truncated: collected.node_walk_truncated,
    })
}

fn language_for_path(path: &Path) -> Option<WorkspaceCodeLanguage> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("rs") {
        return Some(WorkspaceCodeLanguage::Rust);
    }
    if extension.eq_ignore_ascii_case("tsx") {
        return Some(WorkspaceCodeLanguage::Tsx);
    }
    if extension.eq_ignore_ascii_case("ts")
        || extension.eq_ignore_ascii_case("mts")
        || extension.eq_ignore_ascii_case("cts")
    {
        return Some(WorkspaceCodeLanguage::TypeScript);
    }
    None
}

#[derive(Debug)]
struct CollectedSyntaxDiagnostics {
    syntax_error_count: usize,
    nodes_visited: usize,
    diagnostics: Vec<WorkspaceSyntaxDiagnostic>,
    diagnostics_truncated: bool,
    node_walk_truncated: bool,
}

fn collect_syntax_diagnostics(
    root: Node<'_>,
    maximum_nodes: usize,
    maximum_diagnostics: usize,
) -> CollectedSyntaxDiagnostics {
    let mut cursor = root.walk();
    let mut syntax_error_count = 0_usize;
    let mut nodes_visited = 0_usize;
    let mut diagnostics = Vec::new();
    let mut diagnostics_truncated = false;
    let mut node_walk_truncated = false;
    let mut finished = false;

    while !finished {
        if nodes_visited == maximum_nodes {
            node_walk_truncated = true;
            break;
        }

        let node = cursor.node();
        nodes_visited = nodes_visited.saturating_add(1);
        if node.is_error() || node.is_missing() {
            syntax_error_count = syntax_error_count.saturating_add(1);
            if diagnostics.len() < maximum_diagnostics {
                let start = node.start_position();
                let end = node.end_position();
                let kind = if node.is_missing() {
                    WorkspaceSyntaxIssueKind::MissingNode
                } else {
                    WorkspaceSyntaxIssueKind::Error
                };
                diagnostics.push(WorkspaceSyntaxDiagnostic {
                    kind,
                    node_kind: node.kind().to_owned(),
                    start_byte: node.start_byte(),
                    end_byte: node.end_byte(),
                    start_line: start.row.saturating_add(1),
                    start_column_bytes: start.column.saturating_add(1),
                    end_line: end.row.saturating_add(1),
                    end_column_bytes: end.column.saturating_add(1),
                });
            } else {
                diagnostics_truncated = true;
            }
        }

        if cursor.goto_first_child() {
            continue;
        }

        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                finished = true;
                break;
            }
        }
    }

    CollectedSyntaxDiagnostics {
        syntax_error_count,
        nodes_visited,
        diagnostics,
        diagnostics_truncated,
        node_walk_truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_PARSE_DIAGNOSTICS, WorkspaceCodeLanguage, WorkspaceParseError,
        WorkspaceSyntaxIssueKind, collect_syntax_diagnostics, parse_workspace_file,
    };
    use carapana_storage::{WorkspaceFileReadError, WorkspaceMetadata};
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
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "carapana-tools-parser-{}-{nonce}-{id}",
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

    fn create_workspace(directory: &TestDirectory) -> WorkspaceMetadata {
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        WorkspaceMetadata::capture(workspace_path).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn should_parse_supported_languages_on_demand_without_returning_source_text() {
        let directory = TestDirectory::new();
        let workspace = create_workspace(&directory);
        let workspace_path = workspace.path();
        fs::create_dir(workspace_path.join("src")).unwrap();
        fs::write(
            workspace_path.join("src/lib.rs"),
            "pub fn answer() -> i32 { 42 }\n",
        )
        .unwrap();
        fs::write(
            workspace_path.join("src/types.ts"),
            "export const answer: number = 42;\n",
        )
        .unwrap();
        fs::write(
            workspace_path.join("src/view.tsx"),
            "export const View = () => <div />;\n",
        )
        .unwrap();

        let rust = parse_workspace_file(&workspace, "src/lib.rs").unwrap();
        let typescript = parse_workspace_file(&workspace, "src/types.ts").unwrap();
        let tsx = parse_workspace_file(&workspace, "src/view.tsx").unwrap();

        assert_eq!(rust.language, WorkspaceCodeLanguage::Rust);
        assert_eq!(typescript.language, WorkspaceCodeLanguage::TypeScript);
        assert_eq!(tsx.language, WorkspaceCodeLanguage::Tsx);
        for parsed in [&rust, &typescript, &tsx] {
            assert!(!parsed.has_syntax_errors);
            assert!(parsed.diagnostics.is_empty());
        }
        assert!(!format!("{rust:?}{typescript:?}{tsx:?}").contains("answer()"));
    }

    #[cfg(unix)]
    #[test]
    fn should_return_bounded_syntax_error_locations_without_source_snippets() {
        let directory = TestDirectory::new();
        let workspace = create_workspace(&directory);
        let source = (0..(MAX_PARSE_DIAGNOSTICS + 10))
            .map(|index| format!("fn broken_{index}() {{ let value = ; }}\n"))
            .collect::<String>();
        fs::write(workspace.path().join("broken.rs"), &source).unwrap();

        let parsed = parse_workspace_file(&workspace, "broken.rs").unwrap();

        assert!(parsed.has_syntax_errors);
        assert!(!parsed.diagnostics.is_empty());
        assert!(parsed.diagnostics.len() <= MAX_PARSE_DIAGNOSTICS);
        assert!(parsed.diagnostics_truncated);
        assert!(parsed.syntax_error_count > parsed.diagnostics.len());
        assert!(parsed.diagnostics.iter().all(|diagnostic| {
            diagnostic.start_line > 0
                && diagnostic.start_column_bytes > 0
                && diagnostic.end_byte >= diagnostic.start_byte
        }));
        assert!(!format!("{parsed:?}").contains("let value"));
    }

    #[cfg(unix)]
    #[test]
    fn should_report_node_walk_truncation_without_recursing() {
        let directory = TestDirectory::new();
        let workspace = create_workspace(&directory);
        let workspace_path = workspace.path();
        fs::write(
            workspace_path.join("many.rs"),
            "fn main() { let value = 1; }\n",
        )
        .unwrap();
        let source = workspace.read_file("many.rs").unwrap();
        let source = std::str::from_utf8(&source).unwrap();
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();

        let collected = collect_syntax_diagnostics(tree.root_node(), 1, 1);

        assert_eq!(collected.nodes_visited, 1);
        assert!(collected.node_walk_truncated);
        assert!(!collected.diagnostics_truncated);
    }

    #[cfg(unix)]
    #[test]
    fn should_fail_closed_for_unsupported_sensitive_non_utf8_and_oversized_files() {
        let directory = TestDirectory::new();
        let workspace = create_workspace(&directory);
        let workspace_path = workspace.path();
        fs::write(
            workspace_path.join("view.svelte"),
            "<script>let x = 1;</script>",
        )
        .unwrap();
        fs::write(workspace_path.join("secrets.rs"), "fn secret() {}\n").unwrap();
        fs::write(workspace_path.join("invalid.rs"), [0xff, 0xfe]).unwrap();
        fs::write(
            workspace_path.join("large.rs"),
            vec![b'x'; carapana_storage::MAX_WORKSPACE_FILE_READ_BYTES + 1],
        )
        .unwrap();

        assert!(matches!(
            parse_workspace_file(&workspace, "view.svelte"),
            Err(WorkspaceParseError::UnsupportedLanguage)
        ));
        assert!(matches!(
            parse_workspace_file(&workspace, "secrets.rs"),
            Err(WorkspaceParseError::WorkspaceRead(
                WorkspaceFileReadError::SensitivePath(_)
            ))
        ));
        assert!(matches!(
            parse_workspace_file(&workspace, "invalid.rs"),
            Err(WorkspaceParseError::InvalidUtf8)
        ));
        assert!(matches!(
            parse_workspace_file(&workspace, "large.rs"),
            Err(WorkspaceParseError::WorkspaceRead(
                WorkspaceFileReadError::FileTooLarge(_)
            ))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_map_changed_workspace_identity_to_a_read_error() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        let moved_workspace = directory.0.join("moved-workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(workspace_path.join("source.rs"), "fn main() {}\n").unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();
        fs::rename(&workspace_path, &moved_workspace).unwrap();
        fs::create_dir(&workspace_path).unwrap();

        assert!(matches!(
            parse_workspace_file(&workspace, "source.rs"),
            Err(WorkspaceParseError::WorkspaceRead(
                WorkspaceFileReadError::Workspace(_)
            ))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_mark_diagnostic_collection_truncated_when_more_errors_exist() {
        let directory = TestDirectory::new();
        let workspace = create_workspace(&directory);
        let source = (0..10)
            .map(|index| format!("fn broken_{index}() {{ let value = ; }}\n"))
            .collect::<String>();
        fs::write(workspace.path().join("broken.rs"), &source).unwrap();
        let source = workspace.read_file("broken.rs").unwrap();
        let source = std::str::from_utf8(&source).unwrap();
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();

        let collected = collect_syntax_diagnostics(tree.root_node(), usize::MAX, 1);

        assert!(collected.syntax_error_count > 1);
        assert_eq!(collected.diagnostics.len(), 1);
        assert!(collected.diagnostics_truncated);
        assert!(
            collected
                .diagnostics
                .iter()
                .all(
                    |diagnostic| diagnostic.kind == WorkspaceSyntaxIssueKind::Error
                        || diagnostic.kind == WorkspaceSyntaxIssueKind::MissingNode
                )
        );
    }
}
