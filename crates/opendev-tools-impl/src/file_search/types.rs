//! Types and argument parsing for the file search tool.
//!
//! Defines search arguments, output modes, search types, and the
//! ripgrep error enum.

use std::collections::HashMap;

/// Parsed search arguments.
pub(super) struct SearchArgs {
    pub pattern: String,
    pub path: Option<String>,
    pub glob: Option<String>,
    pub file_type: Option<String>,
    pub case_insensitive: bool,
    pub multiline: bool,
    pub fixed_string: bool,
    pub output_mode: OutputMode,
    pub context: Option<u32>,
    pub after_context: Option<u32>,
    pub before_context: Option<u32>,
    pub line_numbers: bool,
    pub head_limit: usize,
    pub offset: usize,
    /// Search type: "text" (default, ripgrep) or "ast" (ast-grep structural search).
    pub search_type: SearchType,
    /// Language hint for AST mode (auto-detected from file extension if not specified).
    pub lang: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SearchType {
    Text,
    Ast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutputMode {
    Content,
    FilesWithMatches,
    Count,
}

impl SearchArgs {
    pub fn from_map(args: &HashMap<String, serde_json::Value>) -> Result<Self, String> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "pattern is required".to_string())?
            .to_string();

        let output_mode = match args.get("output_mode").and_then(|v| v.as_str()) {
            Some("files_with_matches") => OutputMode::FilesWithMatches,
            Some("count") => OutputMode::Count,
            Some("content") | None => OutputMode::Content,
            Some(other) => {
                return Err(format!(
                    "Invalid output_mode '{other}'. Use 'content', 'files_with_matches', or 'count'"
                ));
            }
        };

        let line_numbers = args.get("-n").and_then(|v| v.as_bool()).unwrap_or(true);

        let search_type = match args.get("search_type").and_then(|v| v.as_str()) {
            Some("ast") => SearchType::Ast,
            Some("text") | None => SearchType::Text,
            Some(other) => {
                return Err(format!(
                    "Invalid search_type '{other}'. Use 'text' or 'ast'"
                ));
            }
        };

        let lang = args.get("lang").and_then(|v| v.as_str()).map(String::from);

        Ok(Self {
            pattern,
            path: args.get("path").and_then(|v| v.as_str()).map(String::from),
            glob: args
                .get("glob")
                .or_else(|| args.get("include"))
                .and_then(|v| v.as_str())
                .map(String::from),
            file_type: args
                .get("type")
                .or_else(|| args.get("file_type"))
                .and_then(|v| v.as_str())
                .map(String::from),
            case_insensitive: args.get("-i").and_then(|v| v.as_bool()).unwrap_or(false),
            multiline: args
                .get("multiline")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            fixed_string: args
                .get("fixed_string")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            output_mode,
            context: args
                .get("context")
                .or_else(|| args.get("-C"))
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            after_context: args.get("-A").and_then(|v| v.as_u64()).map(|v| v as u32),
            before_context: args.get("-B").and_then(|v| v.as_u64()).map(|v| v as u32),
            line_numbers,
            head_limit: args.get("head_limit").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            offset: args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            search_type,
            lang,
        })
    }
}

/// Errors that can occur when running ripgrep.
pub(super) enum RgError {
    NotInstalled,
    Timeout,
    Other(String),
}
