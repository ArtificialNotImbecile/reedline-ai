//! Explicit, bounded selection from history supplied by the caller.
//!
//! This module never reads a shell history file, environment variable, or key.
//! Budgets use serialized UTF-8 bytes as a conservative token *estimate*, not
//! an actual tokenizer count or a tokenizer-independent upper-bound guarantee.

use crate::context::{Context, unsafe_char};
use serde::Serialize;
use std::{collections::HashSet, fmt};

pub const DEFAULT_REQUEST_BUDGET: usize = 8192;
pub const EXPANDED_REQUEST_BUDGET: usize = 16384;
pub const DEFAULT_HISTORY_BUDGET: usize = 4096;
pub const OUTPUT_TOKEN_RESERVE: usize = 128;
/// Explicit allowance for chat framing and provider-side protocol overhead.
/// Provider internals can vary, so this is a reserve, not an exact token count.
pub const PROTOCOL_ESTIMATE_RESERVE: usize = 256;
pub const MAX_HISTORY_ENTRIES: usize = 50;
pub const MAX_HISTORY_CANDIDATES: usize = 500;
pub const MAX_HISTORY_ENTRY_BYTES: usize = 1024;

/// Limits in UTF-8-byte estimate units, except the fixed 128 output-token
/// reserve. Increase the total limit to 16384 only through an explicit choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RequestBudget {
    pub total: usize,
    pub history: usize,
}

impl Default for RequestBudget {
    fn default() -> Self {
        Self {
            total: DEFAULT_REQUEST_BUDGET,
            history: DEFAULT_HISTORY_BUDGET,
        }
    }
}

impl RequestBudget {
    pub fn new(total: usize, history: usize) -> Result<Self, BudgetError> {
        let budget = Self { total, history };
        budget.validate()?;
        Ok(budget)
    }

    pub fn validate(self) -> Result<(), BudgetError> {
        if self.total == 0 || self.total > EXPANDED_REQUEST_BUDGET {
            return Err(BudgetError::InvalidConfig(
                "request estimate budget must be between 1 and 16384",
            ));
        }
        if self.history > self.total {
            return Err(BudgetError::InvalidConfig(
                "history estimate budget must not exceed the total request estimate budget",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BudgetError {
    InvalidConfig(&'static str),
    InvalidHistory(&'static str),
    RequestExceeded { estimate: usize, limit: usize },
    HistoryExceeded { estimate: usize, limit: usize },
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) | Self::InvalidHistory(message) => f.write_str(message),
            Self::RequestExceeded { estimate, limit } => write!(
                f,
                "request UTF-8-byte estimate {estimate} exceeds budget {limit}; no request sent"
            ),
            Self::HistoryExceeded { estimate, limit } => write!(
                f,
                "history UTF-8-byte estimate {estimate} exceeds budget {limit}; no request sent"
            ),
        }
    }
}

impl std::error::Error for BudgetError {}

/// A reproducible estimate, not measured provider token usage. Serialized JSON
/// punctuation/escaping, the fixed system prompt, chat protocol reserve, and
/// maximum output allowance are all included. The HTTP provider's usage fields
/// are the only actual token counts exposed by this crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RequestEstimate {
    pub method: &'static str,
    pub system_prompt_bytes: usize,
    pub current_context_bytes: usize,
    pub history_bytes: usize,
    pub protocol_reserve: usize,
    pub output_token_reserve: usize,
    pub total_estimate: usize,
    pub total_budget: usize,
    pub history_budget: usize,
}

/// Report and enforce the same estimate used immediately before an HTTP send.
/// `system_prompt` must be the exact prompt sent by the backend.
pub fn estimate_with_system_prompt(
    context: &Context,
    budget: RequestBudget,
    system_prompt: &str,
) -> Result<RequestEstimate, BudgetError> {
    budget.validate()?;
    let current_context_bytes = serde_json::to_vec(&context.clone().without_history())
        .expect("context serializes")
        .len();
    let context_bytes = serde_json::to_vec(context)
        .expect("context serializes")
        .len();
    let history_bytes = context_bytes - current_context_bytes;
    let total_estimate = system_prompt
        .len()
        .saturating_add(context_bytes)
        .saturating_add(PROTOCOL_ESTIMATE_RESERVE)
        .saturating_add(OUTPUT_TOKEN_RESERVE);
    if history_bytes > budget.history {
        return Err(BudgetError::HistoryExceeded {
            estimate: history_bytes,
            limit: budget.history,
        });
    }
    if total_estimate > budget.total {
        return Err(BudgetError::RequestExceeded {
            estimate: total_estimate,
            limit: budget.total,
        });
    }
    Ok(RequestEstimate {
        method: "utf8_bytes_with_protocol_and_output_reserves_not_actual_tokens",
        system_prompt_bytes: system_prompt.len(),
        current_context_bytes,
        history_bytes,
        protocol_reserve: PROTOCOL_ESTIMATE_RESERVE,
        output_token_reserve: OUTPUT_TOKEN_RESERVE,
        total_estimate,
        total_budget: budget.total,
        history_budget: budget.history,
    })
}

#[derive(Clone, Default)]
pub struct HistoryContextConfig {
    /// Upload context is opt-in. Offline previews can explicitly enable a copy.
    pub enabled: bool,
    pub budget: RequestBudget,
    /// Case-sensitive literal substring exclusions. These are not a secret
    /// detector and must not be represented as making history safe to upload.
    pub exclusions: Vec<String>,
}

impl fmt::Debug for HistoryContextConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryContextConfig")
            .field("enabled", &self.enabled)
            .field("budget", &self.budget)
            .field("exclusion_count", &self.exclusions.len())
            .finish()
    }
}

/// Explicit previews contain user data; debug output deliberately does not.
#[derive(Clone, Serialize)]
pub struct HistoryPreview {
    pub entries: Vec<String>,
    pub estimate: RequestEstimate,
    pub examined: usize,
    pub omitted_invalid: usize,
    pub omitted_excluded: usize,
    pub omitted_duplicates: usize,
    pub omitted_budget: usize,
    pub omitted_limit: usize,
}

impl fmt::Debug for HistoryPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryPreview")
            .field("entry_count", &self.entries.len())
            .field("estimate", &self.estimate)
            .field("examined", &self.examined)
            .finish_non_exhaustive()
    }
}

impl HistoryPreview {
    /// Attach exactly the previewed entries. Backend validation remains
    /// mandatory because the caller may have changed the current input.
    pub fn into_context(self, context: Context) -> Result<Context, BudgetError> {
        context
            .with_history(self.entries)
            .map_err(BudgetError::InvalidHistory)
    }
}

pub fn validate_history_entry(entry: &str) -> Result<(), &'static str> {
    if entry.trim().is_empty() {
        return Err("history entries must not be empty");
    }
    if entry.len() > MAX_HISTORY_ENTRY_BYTES {
        return Err("history entry exceeds the 1024-byte limit");
    }
    if entry.chars().any(unsafe_char) || entry.contains("```") {
        return Err("history entry contains unsafe formatting or terminal controls");
    }
    Ok(())
}

/// Select from at most the latest 500 supplied entries (oldest first).
/// Exact duplicates keep their newest occurrence. Relevant and recent entries
/// alternate, so older relevant commands do not crowd out all recent context.
/// The final result is in chronological order, with at most 50 complete entries.
/// No entry is truncated. Unsafe, excluded, and oversized entries are omitted.
pub fn select_history(
    context: &Context,
    history: &[String],
    config: &HistoryContextConfig,
) -> Result<HistoryPreview, BudgetError> {
    let base = context.clone().without_history();
    let estimate = crate::backend::estimate_request(&base, config.budget)?;
    let mut preview = HistoryPreview {
        entries: Vec::new(),
        estimate,
        examined: 0,
        omitted_invalid: 0,
        omitted_excluded: 0,
        omitted_duplicates: 0,
        omitted_budget: 0,
        omitted_limit: 0,
    };
    if !config.enabled {
        return Ok(preview);
    }
    let query_words: HashSet<_> = base
        .prefix()
        .split_whitespace()
        .chain(base.suffix().split_whitespace())
        .collect();
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for (index, entry) in history
        .iter()
        .enumerate()
        .rev()
        .take(MAX_HISTORY_CANDIDATES)
    {
        preview.examined += 1;
        if validate_history_entry(entry).is_err() {
            preview.omitted_invalid += 1;
        } else if config
            .exclusions
            .iter()
            .any(|excluded| entry.contains(excluded))
        {
            preview.omitted_excluded += 1;
        } else if !seen.insert(entry.as_str()) {
            preview.omitted_duplicates += 1;
        } else {
            candidates.push((index, entry.as_str(), relevance(&base, entry, &query_words)));
        }
    }
    // candidates is newest first. A separate stable ranking favors matches,
    // breaking equal relevance by recency, never by randomized hash iteration.
    let mut relevant: Vec<_> = (0..candidates.len())
        .filter(|&index| candidates[index].2 > 0)
        .collect();
    relevant.sort_by_key(|&index| std::cmp::Reverse((candidates[index].2, candidates[index].0)));
    let mut order = Vec::with_capacity(candidates.len());
    let mut ordered = vec![false; candidates.len()];
    let mut recent_cursor = 0;
    let mut relevant_cursor = 0;
    while order.len() < candidates.len() {
        while relevant_cursor < relevant.len() && ordered[relevant[relevant_cursor]] {
            relevant_cursor += 1;
        }
        if relevant_cursor < relevant.len() {
            let next = relevant[relevant_cursor];
            ordered[next] = true;
            order.push(next);
        }
        while recent_cursor < candidates.len() && ordered[recent_cursor] {
            recent_cursor += 1;
        }
        if recent_cursor < candidates.len() {
            ordered[recent_cursor] = true;
            order.push(recent_cursor);
        }
    }
    let mut selected = Vec::new();
    let mut history_bytes = 0;
    for index in order {
        if selected.len() == MAX_HISTORY_ENTRIES {
            preview.omitted_limit += 1;
            continue;
        }
        // Count only each new JSON string, rather than reserializing the whole
        // current input up to 500 times on every editor update. The first item
        // adds the field and array; subsequent items add a comma. Verify the
        // final exact serialization with the shared backend estimator below.
        let entry_bytes = serde_json::to_vec(candidates[index].1)
            .expect("history string serializes")
            .len();
        let added_bytes = entry_bytes
            + if selected.is_empty() {
                b",\"history\":[]".len()
            } else {
                1
            };
        let trial_history_bytes = history_bytes + added_bytes;
        if trial_history_bytes > config.budget.history
            || preview.estimate.total_estimate + trial_history_bytes > config.budget.total
        {
            preview.omitted_budget += 1;
        } else {
            selected.push(index);
            history_bytes = trial_history_bytes;
        }
    }
    selected.sort_by_key(|&index| candidates[index].0);
    preview.entries = selected
        .into_iter()
        .map(|index| candidates[index].1.to_string())
        .collect();
    let selected_context = base
        .with_history(preview.entries.clone())
        .expect("validated candidates");
    preview.estimate = crate::backend::estimate_request(&selected_context, config.budget)?;
    debug_assert_eq!(preview.estimate.history_bytes, history_bytes);
    Ok(preview)
}

fn relevance(context: &Context, entry: &str, query_words: &HashSet<&str>) -> usize {
    let prefix = context.prefix();
    let suffix = context.suffix();
    if !context.is_empty() && entry.starts_with(prefix) && entry.ends_with(suffix) {
        return 3;
    }
    let first = prefix.split_whitespace().next();
    if first.is_some() && first == entry.split_whitespace().next() {
        return 2;
    }
    if entry
        .split_whitespace()
        .any(|word| query_words.contains(word))
    {
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn enabled() -> HistoryContextConfig {
        HistoryContextConfig {
            enabled: true,
            ..HistoryContextConfig::default()
        }
    }

    fn strings(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| (*entry).to_string()).collect()
    }

    #[test]
    fn defaults_are_opt_in_and_preserve_wire_format() {
        let context = Context::new("git st", 6).unwrap();
        assert_eq!(
            serde_json::to_value(&context).unwrap(),
            json!({"prefix":"git st","suffix":""})
        );
        let preview = select_history(
            &context,
            &strings(&["git status"]),
            &HistoryContextConfig::default(),
        )
        .unwrap();
        assert!(preview.entries.is_empty());
        assert_eq!(preview.examined, 0);
        assert_eq!(preview.estimate.history_bytes, 0);
        assert_eq!(preview.estimate.total_budget, 8192);
        assert_eq!(preview.estimate.history_budget, 4096);
        assert_eq!(preview.estimate.output_token_reserve, 128);
    }

    #[test]
    fn selection_is_deterministic_deduplicated_and_excludable() {
        let context = Context::new("git st", 6).unwrap();
        let history = strings(&["git status", "pwd", "git log", "git status", "echo PRIVATE"]);
        let mut config = enabled();
        config.exclusions.push("PRIVATE".into());
        let one = select_history(&context, &history, &config).unwrap();
        let two = select_history(&context, &history, &config).unwrap();
        assert_eq!(one.entries, strings(&["pwd", "git log", "git status"]));
        assert_eq!(one.entries, two.entries);
        assert_eq!(one.omitted_duplicates, 1);
        assert_eq!(one.omitted_excluded, 1);
        let attached = one.into_context(context.clone()).unwrap();
        assert_eq!(attached.prefix(), context.prefix());
        assert_eq!(attached.suffix(), context.suffix());
        assert_eq!(
            serde_json::to_value(&attached).unwrap()["history"],
            json!(["pwd", "git log", "git status"])
        );
    }

    #[test]
    fn unsafe_and_oversized_entries_are_rejected_not_sanitized() {
        let context = Context::new("git", 3).unwrap();
        let mut history = strings(&[
            "",
            " ",
            "echo\nsecret",
            "echo\tsecret",
            "\u{1b}[31msecret",
            "x\u{202e}y",
            "```sh",
            "echo 世界",
        ]);
        history.push("x".repeat(MAX_HISTORY_ENTRY_BYTES + 1));
        let preview = select_history(&context, &history, &enabled()).unwrap();
        assert_eq!(preview.entries, strings(&["echo 世界"]));
        assert_eq!(preview.omitted_invalid, 8);
        for invalid in &history[..7] {
            assert!(context.clone().with_history(vec![invalid.clone()]).is_err());
        }
        assert!(context.with_history(vec!["pwd".into(); 51]).is_err());
    }

    #[test]
    fn only_latest_500_candidates_and_at_most_50_entries_are_used() {
        let context = Context::new("git", 3).unwrap();
        let history: Vec<_> = (0..700).map(|index| format!("command-{index}")).collect();
        let preview = select_history(&context, &history, &enabled()).unwrap();
        assert_eq!(preview.examined, 500);
        assert_eq!(preview.entries.len(), 50);
        assert_eq!(preview.entries.first().unwrap(), "command-650");
        assert_eq!(preview.entries.last().unwrap(), "command-699");
        assert_eq!(preview.omitted_limit, 450);
    }

    #[test]
    fn relevance_and_recency_both_survive_selection() {
        let context = Context::new("git st", 6).unwrap();
        let mut history: Vec<_> = (0..50)
            .map(|index| format!("git status --example-{index}"))
            .collect();
        history.extend((0..50).map(|index| format!("recent-{index}")));
        let preview = select_history(&context, &history, &enabled()).unwrap();
        assert_eq!(preview.entries.len(), 50);
        assert_eq!(
            preview
                .entries
                .iter()
                .filter(|entry| entry.starts_with("git"))
                .count(),
            25
        );
        assert_eq!(
            preview
                .entries
                .iter()
                .filter(|entry| entry.starts_with("recent"))
                .count(),
            25
        );
    }

    #[test]
    fn serialized_utf8_bytes_and_json_escaping_are_counted() {
        let context = Context::new("echo 世界", "echo 世界".len()).unwrap();
        let with_history = context
            .clone()
            .with_history(strings(&["echo \"🦀\"\\path"]))
            .unwrap();
        let estimate =
            crate::backend::estimate_request(&with_history, RequestBudget::default()).unwrap();
        let current_bytes = serde_json::to_vec(&context).unwrap().len();
        let all_bytes = serde_json::to_vec(&with_history).unwrap().len();
        assert_eq!(estimate.current_context_bytes, current_bytes);
        assert_eq!(estimate.history_bytes, all_bytes - current_bytes);
        assert_eq!(
            estimate.total_estimate,
            crate::backend::SYSTEM_PROMPT.len() + all_bytes + 256 + 128
        );
        assert!(estimate.method.contains("not_actual_tokens"));
    }

    #[test]
    fn history_and_total_limits_are_enforced_without_truncation() {
        let context = Context::new("git st", 6).unwrap();
        let history = strings(&["git status", "pwd", "echo 世界"]);
        let base = crate::backend::estimate_request(&context, RequestBudget::default()).unwrap();
        let mut config = enabled();
        config.budget.history = 25;
        let preview = select_history(&context, &history, &config).unwrap();
        assert!(preview.estimate.history_bytes <= 25);
        assert!(preview.omitted_budget > 0);
        assert!(preview.entries.iter().all(|entry| history.contains(entry)));
        config.budget = RequestBudget::new(base.total_estimate, 25).unwrap();
        let preview = select_history(&context, &history, &config).unwrap();
        assert!(preview.entries.is_empty());
        assert_eq!(preview.omitted_budget, 3);
        config.budget.total -= 1;
        assert!(matches!(
            select_history(&context, &history, &config),
            Err(BudgetError::RequestExceeded { .. })
        ));
    }

    #[test]
    fn expanded_budget_is_explicit_and_invalid_limits_are_errors() {
        let large = Context::new(&"x".repeat(8192), 8192).unwrap();
        assert!(matches!(
            crate::backend::estimate_request(&large, RequestBudget::default()),
            Err(BudgetError::RequestExceeded { .. })
        ));
        assert!(
            crate::backend::estimate_request(&large, RequestBudget::new(16384, 4096).unwrap())
                .is_ok()
        );
        assert!(RequestBudget::new(0, 0).is_err());
        assert!(RequestBudget::new(16385, 4096).is_err());
        assert!(RequestBudget::new(8192, 8193).is_err());
    }

    #[test]
    fn diagnostics_never_include_commands_or_exclusions() {
        let context = Context::new("echo PRIVATE", 12)
            .unwrap()
            .with_history(strings(&["echo HISTORY_SECRET"]))
            .unwrap();
        let config = HistoryContextConfig {
            exclusions: strings(&["EXCLUSION_SECRET"]),
            ..enabled()
        };
        let preview =
            select_history(&context, &strings(&["echo HISTORY_SECRET"]), &config).unwrap();
        let diagnostics = format!("{context:?} {config:?} {preview:?}");
        for secret in ["PRIVATE", "HISTORY_SECRET", "EXCLUSION_SECRET"] {
            assert!(!diagnostics.contains(secret));
        }
    }
}
