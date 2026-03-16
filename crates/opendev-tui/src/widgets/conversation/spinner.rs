//! Spinner and progress line rendering for active tools and subagents.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::formatters::style_tokens;
use crate::formatters::tool_registry::format_tool_call_parts_with_wd;
use crate::widgets::spinner::{COMPACTION_CHAR, COMPLETED_CHAR, CONTINUATION_CHAR, SPINNER_FRAMES};

use super::ConversationWidget;

impl<'a> ConversationWidget<'a> {
    /// Build spinner/progress lines separately from message content.
    ///
    /// These are rendered outside the scrollable area so that spinner
    /// animation (60ms ticks) doesn't shift scroll math or cause jitter.
    pub(super) fn build_spinner_lines(&self) -> Vec<Line<'a>> {
        let mut lines: Vec<Line> = Vec::new();

        let active_unfinished: Vec<_> = self
            .active_tools
            .iter()
            .filter(|t| !t.is_finished())
            .collect();

        if self.compaction_active {
            // Compaction spinner: ✻ Compacting conversation…
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", COMPACTION_CHAR),
                    Style::default()
                        .fg(style_tokens::BLUE_BRIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Compacting conversation\u{2026}",
                    Style::default()
                        .fg(style_tokens::SUBTLE)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        } else if !active_unfinished.is_empty() {
            for tool in &active_unfinished {
                let frame_idx = tool.tick_count % SPINNER_FRAMES.len();
                let spinner = SPINNER_FRAMES[frame_idx];

                // For spawn_subagent, use Python-style nested display
                if tool.name == "spawn_subagent" {
                    // Match by task text: each subagent has a unique task string.
                    // Don't filter by !finished — we need to show subagents in the gap
                    // between SubagentFinished and ToolFinished events.
                    let tool_task = tool.args.get("task").and_then(|v| v.as_str()).unwrap_or("");
                    let subagent = self.active_subagents.iter().find(|s| s.task == tool_task);
                    let (agent_name, task_desc) = if let Some(sa) = subagent {
                        (sa.name.clone(), sa.task.clone())
                    } else {
                        let name = tool
                            .args
                            .get("agent_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Agent");
                        let task = tool.args.get("task").and_then(|v| v.as_str()).unwrap_or("");
                        (name.to_string(), task.to_string())
                    };

                    let task_short = if task_desc.len() > 60 {
                        format!("{}...", &task_desc[..60])
                    } else {
                        task_desc
                    };

                    // Check if multiple parallel subagents are active
                    let spawn_count = active_unfinished
                        .iter()
                        .filter(|t| t.name == "spawn_subagent")
                        .count();
                    let is_parallel = spawn_count > 1;

                    // Header: ⠋ AgentName(task description)
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{spinner} "),
                            Style::default().fg(style_tokens::BLUE_BRIGHT),
                        ),
                        Span::styled(
                            agent_name,
                            Style::default()
                                .fg(style_tokens::PRIMARY)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("({task_short})"),
                            Style::default().fg(style_tokens::SUBTLE),
                        ),
                    ]));

                    if let Some(sa) = subagent {
                        if is_parallel {
                            self.build_parallel_subagent_line(sa, &mut lines);
                        } else if sa.finished {
                            // Single subagent finished but tool not yet — show Done summary
                            let tool_count = sa.tool_call_count;
                            let count_str = if tool_count > 0 {
                                format!(" ({tool_count} tool uses)")
                            } else {
                                String::new()
                            };
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("  {CONTINUATION_CHAR}  "),
                                    Style::default().fg(style_tokens::GREY),
                                ),
                                Span::styled(
                                    format!("{COMPLETED_CHAR} "),
                                    Style::default().fg(style_tokens::GREEN_BRIGHT),
                                ),
                                Span::styled("Done", Style::default().fg(style_tokens::SUBTLE)),
                                Span::styled(
                                    count_str,
                                    Style::default()
                                        .fg(style_tokens::GREY)
                                        .add_modifier(Modifier::ITALIC),
                                ),
                            ]));
                        } else {
                            // Single subagent: show full nested tool calls
                            self.build_single_subagent_lines(sa, &mut lines);
                        }
                    }
                } else {
                    // Normal tool: ⠋ verb(arg) (Xs)
                    let (verb, arg) = format_tool_call_parts_with_wd(
                        &tool.name,
                        &tool.args,
                        Some(self.working_dir),
                    );
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{spinner} "),
                            Style::default().fg(style_tokens::BLUE_BRIGHT),
                        ),
                        Span::styled(
                            verb,
                            Style::default()
                                .fg(style_tokens::PRIMARY)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("({arg})"),
                            Style::default().fg(style_tokens::SUBTLE),
                        ),
                        Span::styled(
                            format!(" ({}s)", tool.elapsed_secs),
                            Style::default().fg(style_tokens::GREY),
                        ),
                    ]));
                }
            }
        } else if let Some(progress) = self.task_progress {
            let elapsed = progress.started_at.elapsed().as_secs();
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", self.spinner_char),
                    Style::default().fg(style_tokens::BLUE_BRIGHT),
                ),
                Span::styled(
                    format!("{}... ", progress.description),
                    Style::default().fg(style_tokens::SUBTLE),
                ),
                Span::styled(
                    format!("({}s \u{00b7} esc to interrupt)", elapsed),
                    Style::default().fg(style_tokens::SUBTLE),
                ),
            ]));
        }

        lines
    }

    /// Build status line for a parallel subagent (finished/running/initializing).
    fn build_parallel_subagent_line(
        &self,
        sa: &crate::widgets::nested_tool::SubagentDisplayState,
        lines: &mut Vec<Line<'a>>,
    ) {
        if sa.finished {
            // Subagent finished but tool not yet — show Done summary
            let tool_count = sa.tool_call_count;
            let count_str = if tool_count > 0 {
                format!(" ({tool_count} tool uses)")
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {CONTINUATION_CHAR}  "),
                    Style::default().fg(style_tokens::GREY),
                ),
                Span::styled(
                    format!("{COMPLETED_CHAR} "),
                    Style::default().fg(style_tokens::GREEN_BRIGHT),
                ),
                Span::styled("Done", Style::default().fg(style_tokens::SUBTLE)),
                Span::styled(
                    count_str,
                    Style::default()
                        .fg(style_tokens::GREY)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        } else if let Some(at) = sa.active_tools.values().next() {
            let at_idx = at.tick % SPINNER_FRAMES.len();
            let at_ch = SPINNER_FRAMES[at_idx];
            let (verb, arg) =
                format_tool_call_parts_with_wd(&at.tool_name, &at.args, Some(self.working_dir));
            let count_str = if sa.completed_tools.is_empty() {
                String::new()
            } else {
                format!(" +{} more", sa.completed_tools.len())
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {CONTINUATION_CHAR}  "),
                    Style::default().fg(style_tokens::GREY),
                ),
                Span::styled(
                    format!("{at_ch} "),
                    Style::default().fg(style_tokens::BLUE_BRIGHT),
                ),
                Span::styled(verb, Style::default().fg(style_tokens::SUBTLE)),
                Span::styled(format!("({arg})"), Style::default().fg(style_tokens::GREY)),
                Span::styled(
                    count_str,
                    Style::default()
                        .fg(style_tokens::GREY)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        } else if let Some(ct) = sa.completed_tools.last() {
            let (icon, color) = if ct.success {
                (COMPLETED_CHAR, style_tokens::GREEN_BRIGHT)
            } else {
                ('\u{2717}', style_tokens::ERROR)
            };
            let (verb, arg) =
                format_tool_call_parts_with_wd(&ct.tool_name, &ct.args, Some(self.working_dir));
            let hidden = sa.completed_tools.len().saturating_sub(1);
            let count_str = if hidden > 0 {
                format!(" +{hidden} more")
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {CONTINUATION_CHAR}  "),
                    Style::default().fg(style_tokens::GREY),
                ),
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(verb, Style::default().fg(style_tokens::SUBTLE)),
                Span::styled(format!("({arg})"), Style::default().fg(style_tokens::GREY)),
                Span::styled(
                    count_str,
                    Style::default()
                        .fg(style_tokens::GREY)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {CONTINUATION_CHAR}  "),
                    Style::default().fg(style_tokens::GREY),
                ),
                Span::styled(
                    "Initializing\u{2026}",
                    Style::default()
                        .fg(style_tokens::SUBTLE)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }

    /// Build status lines for a single (non-parallel) subagent showing nested tool calls.
    fn build_single_subagent_lines(
        &self,
        sa: &crate::widgets::nested_tool::SubagentDisplayState,
        lines: &mut Vec<Line<'a>>,
    ) {
        let start = sa.completed_tools.len().saturating_sub(3);
        for ct in &sa.completed_tools[start..] {
            let (icon, color) = if ct.success {
                (COMPLETED_CHAR, style_tokens::GREEN_BRIGHT)
            } else {
                ('\u{2717}', style_tokens::ERROR)
            };
            let (verb, arg) =
                format_tool_call_parts_with_wd(&ct.tool_name, &ct.args, Some(self.working_dir));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {CONTINUATION_CHAR}  "),
                    Style::default().fg(style_tokens::GREY),
                ),
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(verb, Style::default().fg(style_tokens::SUBTLE)),
                Span::styled(format!("({arg})"), Style::default().fg(style_tokens::GREY)),
            ]));
        }

        for at in sa.active_tools.values() {
            let at_idx = at.tick % SPINNER_FRAMES.len();
            let at_ch = SPINNER_FRAMES[at_idx];
            let (verb, arg) =
                format_tool_call_parts_with_wd(&at.tool_name, &at.args, Some(self.working_dir));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {CONTINUATION_CHAR}  "),
                    Style::default().fg(style_tokens::GREY),
                ),
                Span::styled(
                    format!("{at_ch} "),
                    Style::default().fg(style_tokens::BLUE_BRIGHT),
                ),
                Span::styled(verb, Style::default().fg(style_tokens::SUBTLE)),
                Span::styled(format!("({arg})"), Style::default().fg(style_tokens::GREY)),
            ]));
        }

        if sa.active_tools.is_empty() && sa.completed_tools.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {CONTINUATION_CHAR}  "),
                    Style::default().fg(style_tokens::GREY),
                ),
                Span::styled(
                    "Initializing\u{2026}",
                    Style::default()
                        .fg(style_tokens::SUBTLE)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }

        let hidden = sa.completed_tools.len().saturating_sub(3);
        if hidden > 0 {
            lines.push(Line::from(Span::styled(
                format!("      +{hidden} more tool uses"),
                Style::default()
                    .fg(style_tokens::GREY)
                    .add_modifier(Modifier::ITALIC),
            )));
        }
    }
}
