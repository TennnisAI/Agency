//! Token and cost accounting, read from a coding agent's own transcript.
//!
//! Agents write a JSONL transcript per session under a directory named after
//! the working directory. Every assistant record carries the token usage the
//! provider billed for that turn, so reading the transcript gives real numbers
//! for **interactive** sessions, not just headless ones. Parsing an agent's
//! `--print` JSON output would only ever cover the latter, which is a small
//! fraction of what a run actually spends.
//!
//! Everything here is pure: paths in, numbers out, no clock and no I/O beyond
//! reading files the caller names. The polling and the ledger live in the app.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where an agent keeps its per-directory session transcripts.
///
/// `home` is injected rather than read from the environment so this is
/// testable. `command` is the agent's launch command; only the basename is
/// matched, since a profile may carry an absolute path.
///
/// Shares its encodings with [`crate::usage::session_dir`]'s only other
/// consumer, the app's resume probe, which asks the same "where does this
/// agent keep state for this worktree" question for a different reason.
pub fn session_dir(home: &Path, command: &str, worktree: &Path) -> Option<PathBuf> {
    match format_for(command)? {
        Format::Claude => Some(home.join(".claude").join("projects").join(claude_enc(worktree))),
        Format::Pi => Some(home.join(".pi").join("agent").join("sessions").join(pi_enc(worktree))),
    }
}

/// Claude encodes a cwd by replacing every '/' and '.' with '-'.
pub fn claude_enc(worktree: &Path) -> String {
    worktree.to_string_lossy().chars().map(|c| if c == '/' || c == '.' { '-' } else { c }).collect()
}

/// Pi strips a leading '/', replaces '/' with '-' (dots kept), wraps in '--'..'--'.
pub fn pi_enc(worktree: &Path) -> String {
    let s = worktree.to_string_lossy();
    let s = s.strip_prefix('/').unwrap_or(&s);
    let inner: String = s.chars().map(|c| if c == '/' { '-' } else { c }).collect();
    format!("--{inner}--")
}

/// The transcript dialect an agent writes. The two differ in every field name
/// that matters, so a parse has to know which it is holding rather than sniff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Claude: `message.usage.input_tokens`, a nested `cache_creation` object
    /// carrying the 5m/1h split, and a provider `requestId` per turn.
    Claude,
    /// Pi: `message.usage.input`, a flat `cacheWrite` with no time-to-live
    /// split, and the record's own `id` as the only turn identity.
    Pi,
}

/// Which dialect this agent writes, or `None` if Agency cannot account for its
/// spend at all.
///
/// This is the single default-deny list of agents we can read, and both
/// [`session_dir`] and the parser branch off it. Only agents whose transcript
/// format has actually been read against real files are listed. For everyone
/// else the UI shows nothing rather than zero, because "0 tokens" and "we
/// cannot see this agent's tokens" are different claims and only one of them
/// is true.
///
/// Only the basename is matched, since a profile may carry an absolute path.
pub fn format_for(command: &str) -> Option<Format> {
    let base = Path::new(command).file_name().and_then(|s| s.to_str()).unwrap_or(command);
    match base {
        "claude" => Some(Format::Claude),
        "pi" => Some(Format::Pi),
        _ => None,
    }
}

/// Tokens billed for one assistant turn, split by how they are priced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    /// Cache writes with the default (5 minute) time to live.
    pub cache_write_5m: u64,
    /// Cache writes with the extended (1 hour) time to live. Priced higher
    /// than the 5m variant, so the two cannot be summed before pricing.
    pub cache_write_1h: u64,
    pub cache_read: u64,
}

impl Tokens {
    /// Every token the turn touched. For display only; never price with this.
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }

    fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_write_5m += o.cache_write_5m;
        self.cache_write_1h += o.cache_write_1h;
        self.cache_read += o.cache_read;
    }
}

/// Per-million-token prices in whole cents, avoiding float rounding drift over
/// the tens of thousands of records a long-lived worktree accumulates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rates {
    input: u64,
    output: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
    cache_read: u64,
}

/// Published list prices, per million tokens, in cents.
///
/// Matching is by prefix on the model id, because the ids carry suffixes we do
/// not want to enumerate. An id that matches nothing prices as `None` and the
/// caller shows tokens without a cost, which is the honest answer: a wrong
/// price is worse than no price, and this table cannot know about a model
/// released after this build.
fn rates_for(model: &str) -> Option<Rates> {
    // Cache writes are the base input rate scaled: 1.25x at 5 minutes, 2x at
    // one hour. Cache reads are 0.1x. Encoded literally rather than computed
    // so a future model that breaks the ratio can just be given its own row.
    const fn tier(input: u64, output: u64) -> Rates {
        Rates {
            input,
            output,
            cache_write_5m: input * 5 / 4,
            cache_write_1h: input * 2,
            cache_read: input / 10,
        }
    }
    let opus = tier(500, 2500);
    let sonnet = tier(300, 1500);
    let haiku = tier(100, 500);

    // Longest, most specific prefixes first.
    for (prefix, rates) in [
        ("claude-opus-", opus),
        ("claude-fable-", sonnet),
        ("claude-sonnet-", sonnet),
        ("claude-haiku-", haiku),
        ("claude-3-5-haiku", haiku),
        ("claude-3-opus", opus),
    ] {
        if model.starts_with(prefix) {
            return Some(rates);
        }
    }
    None
}

/// Whether a cost figure can be produced for this model id.
pub fn model_priced(model: &str) -> bool {
    rates_for(model).is_some()
}

fn cost_millicents(t: &Tokens, r: &Rates) -> u64 {
    // Rates are cents per million tokens; work in millicents so a single cheap
    // turn does not round away to zero before it is summed.
    let per = |n: u64, rate: u64| n * rate / 1_000;
    per(t.input, r.input)
        + per(t.output, r.output)
        + per(t.cache_write_5m, r.cache_write_5m)
        + per(t.cache_write_1h, r.cache_write_1h)
        + per(t.cache_read, r.cache_read)
}

/// Usage totalled over a set of transcript records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub tokens: Tokens,
    /// Cost in thousandths of a cent, summed only over records whose model is
    /// in the price table.
    pub millicents: u64,
    /// Records whose model had no price. Non-zero means `millicents`
    /// understates the real spend, and the UI must not present it as a total.
    pub unpriced_records: u64,
    pub records: u64,
}

impl Usage {
    fn add_record(&mut self, model: &str, tokens: &Tokens) {
        self.tokens.add(tokens);
        self.records += 1;
        match rates_for(model) {
            Some(r) => self.millicents += cost_millicents(tokens, &r),
            None => self.unpriced_records += 1,
        }
    }

    pub fn merge(&mut self, other: &Usage) {
        self.tokens.add(&other.tokens);
        self.millicents += other.millicents;
        self.unpriced_records += other.unpriced_records;
        self.records += other.records;
    }

    /// True when every record counted carried a known price.
    pub fn cost_is_complete(&self) -> bool {
        self.unpriced_records == 0
    }

    /// Cost in whole cents, rounded to nearest. `None` when nothing was
    /// priceable, so the caller renders tokens alone rather than "$0.00".
    pub fn cents(&self) -> Option<u64> {
        if self.records == 0 || self.records == self.unpriced_records {
            return None;
        }
        Some((self.millicents + 500) / 1_000)
    }
}

/// What the UI sees on a run: flat counts plus a cost that is allowed to be
/// absent. Cache writes are summed here because the 5m/1h split matters to
/// pricing and not to a reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageInfo {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_tokens: u64,
    /// `None` when no record carried a price we know. The UI then shows
    /// tokens alone rather than inventing a figure.
    pub cents: Option<u64>,
    /// False when some records had no price, so `cents` is a floor rather than
    /// a total and must not be labelled as one.
    pub cost_complete: bool,
}

impl From<&Usage> for UsageInfo {
    fn from(u: &Usage) -> UsageInfo {
        UsageInfo {
            input_tokens: u.tokens.input,
            output_tokens: u.tokens.output,
            cache_write_tokens: u.tokens.cache_write_5m + u.tokens.cache_write_1h,
            cache_read_tokens: u.tokens.cache_read,
            total_tokens: u.tokens.total(),
            cents: u.cents(),
            cost_complete: u.cost_is_complete(),
        }
    }
}

/// One parsed assistant record, before de-duplication.
struct Record {
    /// Provider-side identity of the request. Duplicated across records.
    key: String,
    model: String,
    tokens: Tokens,
}

fn parse_u64(v: Option<&serde_json::Value>) -> u64 {
    v.and_then(|v| v.as_u64()).unwrap_or(0)
}

/// Parse one JSONL line. Returns `None` for anything that is not a billable
/// assistant turn, including malformed lines: a transcript is an append-only
/// log that may be read mid-write, so a truncated final line is normal and
/// must never fail the whole pass.
///
/// In both dialects the presence of `message.usage` is what marks an assistant
/// turn; a user record carries no usage, so no role check is needed.
fn parse_line(line: &str, format: Format) -> Option<Record> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match format {
        Format::Claude => parse_claude_line(&v),
        Format::Pi => parse_pi_line(&v),
    }
}

/// Pi's record shape, read from real session files: usage keys are `input`,
/// `output`, `cacheRead` and `cacheWrite`, and the model id sits on the
/// message the same way Claude's does.
///
/// Pi also writes a `cost` object it computed itself, which is deliberately
/// ignored. Mixing a self-reported figure with the table below would leave one
/// displayed total sourced from two different pricing authorities, and a model
/// the table does not know stays honestly costless. The trade is that a pi run
/// on a non-Anthropic provider shows tokens and no cost even though pi knew
/// the price; see AGE-136.
fn parse_pi_line(v: &serde_json::Value) -> Option<Record> {
    let message = v.get("message")?;
    let usage = message.get("usage")?;

    // Pi carries no provider request id, so the record's own id is the only
    // turn identity available. It is safe as a de-duplication key either way:
    // if pi never repeats a record the key is unique and the dedupe is a
    // no-op, and if it ever rewrites one in place the largest-output rule
    // below settles it exactly as it does for Claude.
    let key = v.get("id").and_then(|r| r.as_str())?.to_string();
    let model = message.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();

    Some(Record {
        key,
        model,
        tokens: Tokens {
            input: parse_u64(usage.get("input")),
            output: parse_u64(usage.get("output")),
            // Pi records one flat cacheWrite with no time-to-live split, so it
            // prices at the 5m rate, the same fallback the flat Claude field
            // takes. Charging the cheaper of the two beats dropping it.
            cache_write_5m: parse_u64(usage.get("cacheWrite")),
            cache_write_1h: 0,
            cache_read: parse_u64(usage.get("cacheRead")),
        },
    })
}

fn parse_claude_line(v: &serde_json::Value) -> Option<Record> {
    let message = v.get("message")?;
    let usage = message.get("usage")?;

    // requestId is the provider's request identity and maps one-to-one with
    // message.id (verified across 17,441 records; zero requestIds spanned two
    // message ids). A handful of records carry no requestId, so fall back to
    // message.id, which was present on every record seen.
    let key = v
        .get("requestId")
        .and_then(|r| r.as_str())
        .or_else(|| message.get("id").and_then(|r| r.as_str()))?
        .to_string();

    let model = message.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();

    // The 5m/1h split lives in a nested object; the flat
    // cache_creation_input_tokens is their sum. Prefer the split, since the
    // two are priced differently, and fall back to charging the whole flat
    // figure at the 5m rate rather than dropping it.
    let (w5, w1) = match usage.get("cache_creation") {
        Some(c) => (
            parse_u64(c.get("ephemeral_5m_input_tokens")),
            parse_u64(c.get("ephemeral_1h_input_tokens")),
        ),
        None => (parse_u64(usage.get("cache_creation_input_tokens")), 0),
    };

    Some(Record {
        key,
        model,
        tokens: Tokens {
            input: parse_u64(usage.get("input_tokens")),
            output: parse_u64(usage.get("output_tokens")),
            cache_write_5m: w5,
            cache_write_1h: w1,
            cache_read: parse_u64(usage.get("cache_read_input_tokens")),
        },
    })
}

/// Total the usage in one transcript's text.
///
/// **De-duplication is not optional here.** Measured over 400 real
/// transcripts, 16,357 of 34,979 usage records were repeats of a request
/// already counted, and 390 of the 400 files contained at least one. Summing
/// the file naively overstates spend by roughly 90%.
///
/// Repeats are almost always byte-identical, but not always: one observed
/// group held the same request twice with `output_tokens` of 68 and then 415,
/// a partial write followed by the settled figure. So the rule is
/// last-write-wins on the largest output, not first-seen.
pub fn parse_transcript(text: &str, format: Format) -> Usage {
    let mut by_key: HashMap<String, Record> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for line in text.lines() {
        let Some(rec) = parse_line(line, format) else { continue };
        match by_key.get_mut(&rec.key) {
            Some(existing) => {
                if rec.tokens.output > existing.tokens.output {
                    *existing = rec;
                }
            }
            None => {
                order.push(rec.key.clone());
                by_key.insert(rec.key.clone(), rec);
            }
        }
    }

    let mut usage = Usage::default();
    for key in &order {
        if let Some(rec) = by_key.get(key) {
            usage.add_record(&rec.model, &rec.tokens);
        }
    }
    usage
}

/// What was known about one transcript file the last time it was read, so an
/// unchanged file can be skipped without opening it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    pub len: u64,
    pub mtime_ms: i64,
}

/// Cached per-directory accounting. Transcripts are append-only and a busy
/// worktree accumulates megabytes of them, so re-reading every file on a 2s
/// poll would be wasteful. A file is re-read only when its length or mtime
/// moved.
#[derive(Debug, Clone, Default)]
pub struct UsageCache {
    files: HashMap<PathBuf, (FileStamp, Usage)>,
}

impl UsageCache {
    pub fn new() -> UsageCache {
        UsageCache::default()
    }

    /// Re-read whatever changed under `dir` and return the directory total.
    ///
    /// A missing or unreadable directory is not an error: the agent may not
    /// have written anything yet, or may not be one we can account for.
    pub fn refresh(&mut self, dir: &Path, format: Format) -> Usage {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Usage::default();
        };

        let mut present: Vec<PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let stamp = FileStamp {
                len: meta.len(),
                mtime_ms: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
            };
            present.push(path.clone());

            let unchanged = self.files.get(&path).is_some_and(|(s, _)| *s == stamp);
            if unchanged {
                continue;
            }
            // A read failure leaves any previous entry in place rather than
            // zeroing it: a transient EBUSY should not make the number jump.
            if let Ok(text) = std::fs::read_to_string(&path) {
                self.files.insert(path, (stamp, parse_transcript(&text, format)));
            }
        }

        // Drop files that have gone, so a cleared transcript directory does
        // not keep reporting spend forever.
        self.files.retain(|p, _| present.contains(p));

        let mut total = Usage::default();
        for (_, usage) in self.files.values() {
            total.merge(usage);
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn rec(req: &str, model: &str, input: u64, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"{req}","message":{{"id":"msg_{req}","model":"{model}","usage":{{"input_tokens":{input},"output_tokens":{output}}}}}}}"#
        )
    }

    #[test]
    fn claude_encoding_matches_the_real_layout() {
        let wt = Path::new("/Users/x/agency/.agency/worktrees/agent-abcd");
        assert_eq!(claude_enc(wt), "-Users-x-agency--agency-worktrees-agent-abcd");
        let home = Path::new("/home");
        assert_eq!(
            session_dir(home, "claude", wt).unwrap(),
            home.join(".claude").join("projects").join(claude_enc(wt))
        );
    }

    #[test]
    fn pi_encoding_keeps_dots_and_wraps() {
        assert_eq!(pi_enc(Path::new("/Users/x/agency")), "--Users-x-agency--");
    }

    #[test]
    fn session_dir_resolves_by_basename_and_skips_unknown_agents() {
        let home = Path::new("/home");
        let wt = Path::new("/w");
        assert!(session_dir(home, "/opt/bin/claude", wt).is_some());
        assert!(session_dir(home, "opencode", wt).is_none());
    }

    #[test]
    fn only_agents_whose_format_we_read_are_supported() {
        assert_eq!(format_for("claude"), Some(Format::Claude));
        assert_eq!(format_for("/usr/local/bin/claude"), Some(Format::Claude));
        assert_eq!(format_for("pi"), Some(Format::Pi));
        // The other eight agents render nothing rather than a confident zero.
        assert_eq!(format_for("codex"), None);
        assert_eq!(format_for("opencode"), None);
    }

    /// Pi's real record shape, copied from a live session file: usage keys
    /// are unsuffixed, the cache write is flat, and the turn identity is the
    /// record's own top-level id.
    fn pi_rec(id: &str, model: &str, input: u64, output: u64) -> String {
        format!(
            r#"{{"type":"message","id":"{id}","parentId":null,"message":{{"role":"assistant","provider":"anthropic","model":"{model}","usage":{{"input":{input},"output":{output},"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{{"total":0}}}}}}}}"#
        )
    }

    #[test]
    fn sums_a_pi_transcript() {
        let text = [pi_rec("a", "claude-opus-4-8", 10, 20), pi_rec("b", "claude-opus-4-8", 5, 1)]
            .join("\n");
        let u = parse_transcript(&text, Format::Pi);
        assert_eq!(u.records, 2);
        assert_eq!(u.tokens.input, 15);
        assert_eq!(u.tokens.output, 21);
        // Pi's model ids match the same price table by prefix.
        assert!(u.cents().is_some());
    }

    #[test]
    fn pi_cache_fields_are_read_and_the_write_prices_at_the_5m_rate() {
        let line = r#"{"type":"message","id":"a","message":{"role":"assistant","model":"claude-opus-4-8","usage":{"input":1,"output":2,"cacheRead":700,"cacheWrite":300}}}"#;
        let u = parse_transcript(line, Format::Pi);
        assert_eq!(u.tokens.cache_read, 700);
        // Pi records no time-to-live split, so the whole write lands on 5m
        // rather than being dropped.
        assert_eq!(u.tokens.cache_write_5m, 300);
        assert_eq!(u.tokens.cache_write_1h, 0);
    }

    #[test]
    fn a_pi_record_repeated_settles_on_the_larger_output() {
        // Pi has no requestId, so the record id carries the dedupe. If pi
        // never repeats a record this is simply a no-op.
        let text = [pi_rec("a", "claude-opus-4-8", 2, 68), pi_rec("a", "claude-opus-4-8", 2, 415)]
            .join("\n");
        let u = parse_transcript(&text, Format::Pi);
        assert_eq!(u.records, 1);
        assert_eq!(u.tokens.output, 415);
    }

    #[test]
    fn pi_non_billable_and_malformed_lines_are_skipped() {
        // The session header, a model_change and a user turn all lack usage;
        // only the assistant record counts.
        let text = [
            r#"{"type":"session","version":3,"id":"s","cwd":"/w"}"#,
            r#"{"type":"model_change","id":"m","provider":"anthropic","modelId":"claude-opus-4-8"}"#,
            r#"{"type":"message","id":"u","message":{"role":"user","content":[{"type":"text","text":"help"}]}}"#,
            "not json at all",
            &pi_rec("a", "claude-opus-4-8", 1, 1),
        ]
        .join("\n");
        assert_eq!(parse_transcript(&text, Format::Pi).records, 1);
    }

    #[test]
    fn a_pi_error_turn_bills_nothing_and_so_shows_nothing() {
        // Observed live: a request rejected by the provider is still written
        // as an assistant record, with every usage figure zero. It must not
        // manufacture a run that looks like it spent something.
        let line = r#"{"type":"message","id":"a","message":{"role":"assistant","model":"claude-opus-4-8","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"total":0}},"stopReason":"error"}}"#;
        let u = parse_transcript(line, Format::Pi);
        assert_eq!(u.tokens.total(), 0);
        assert_eq!(u.cents(), Some(0));
    }

    #[test]
    fn a_pi_model_we_have_no_price_for_yields_tokens_and_no_cost() {
        // Pi can drive non-Anthropic providers, whose ids the table does not
        // carry. Pi writes its own cost figure for those and we still decline
        // to show one, rather than mixing two pricing authorities.
        let u = parse_transcript(&pi_rec("a", "gpt-5", 100, 200), Format::Pi);
        assert_eq!(u.tokens.input, 100);
        assert_eq!(u.cents(), None);
        assert!(!u.cost_is_complete());
    }

    #[test]
    fn the_two_dialects_do_not_read_each_others_records() {
        // A Claude record has no unsuffixed `input`, and a pi record has no
        // `input_tokens`, so a format mix-up yields zeros rather than a
        // plausible wrong number. Reading the wrong dialect is a bug; this
        // pins that it cannot silently half-succeed.
        let claude = rec("a", "claude-opus-5", 10, 20);
        assert_eq!(parse_transcript(&claude, Format::Pi).tokens.total(), 0);
        let pi = pi_rec("a", "claude-opus-4-8", 10, 20);
        assert_eq!(parse_transcript(&pi, Format::Claude).tokens.total(), 0);
    }

    #[test]
    fn sums_a_plain_transcript() {
        let text = [rec("a", "claude-opus-5", 10, 20), rec("b", "claude-opus-5", 5, 1)].join("\n");
        let u = parse_transcript(&text, Format::Claude);
        assert_eq!(u.records, 2);
        assert_eq!(u.tokens.input, 15);
        assert_eq!(u.tokens.output, 21);
    }

    #[test]
    fn identical_repeats_are_counted_once() {
        // 47% of records in real transcripts are repeats; counting them twice
        // nearly doubles the reported spend.
        let one = rec("a", "claude-opus-5", 10, 20);
        let text = [one.clone(), one.clone(), one].join("\n");
        let u = parse_transcript(&text, Format::Claude);
        assert_eq!(u.records, 1);
        assert_eq!(u.tokens.output, 20);
    }

    #[test]
    fn a_partial_repeat_settles_on_the_larger_output() {
        // Observed live: one request written twice, output_tokens 68 then 415.
        let text = [rec("a", "claude-opus-5", 2, 68), rec("a", "claude-opus-5", 2, 415)].join("\n");
        assert_eq!(parse_transcript(&text, Format::Claude).tokens.output, 415);
        // Order must not matter; the larger figure wins either way.
        let text = [rec("a", "claude-opus-5", 2, 415), rec("a", "claude-opus-5", 2, 68)].join("\n");
        assert_eq!(parse_transcript(&text, Format::Claude).tokens.output, 415);
    }

    #[test]
    fn falls_back_to_message_id_when_request_id_is_absent() {
        let line = r#"{"type":"assistant","message":{"id":"msg_x","model":"claude-opus-5","usage":{"input_tokens":1,"output_tokens":2}}}"#;
        let u = parse_transcript(&[line, line].join("\n"), Format::Claude);
        assert_eq!(u.records, 1);
    }

    #[test]
    fn non_billable_and_malformed_lines_are_skipped() {
        // A transcript is read while it is being appended to, so a truncated
        // last line is normal and must not fail the pass.
        let text = [
            r#"{"type":"user","message":{"role":"user"}}"#,
            "not json at all",
            r#"{"type":"assistant","message":{"id":"m","model":"claude-opus-5","usage":{"inp"#,
            &rec("a", "claude-opus-5", 1, 1),
        ]
        .join("\n");
        let u = parse_transcript(&text, Format::Claude);
        assert_eq!(u.records, 1);
    }

    #[test]
    fn cache_writes_split_by_ttl_and_are_priced_apart() {
        let line = r#"{"type":"assistant","requestId":"a","message":{"id":"m","model":"claude-opus-5","usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":7,"cache_creation_input_tokens":30,"cache_creation":{"ephemeral_5m_input_tokens":10,"ephemeral_1h_input_tokens":20}}}}"#;
        let u = parse_transcript(line, Format::Claude);
        assert_eq!(u.tokens.cache_write_5m, 10);
        assert_eq!(u.tokens.cache_write_1h, 20);
        assert_eq!(u.tokens.cache_read, 7);
        // 1h writes cost more than 5m writes, so the split must survive into
        // the price. Same token count at each ttl, different contribution.
        let r = rates_for("claude-opus-5").unwrap();
        assert!(r.cache_write_1h > r.cache_write_5m);
    }

    #[test]
    fn flat_cache_creation_still_counts_when_the_split_is_absent() {
        let line = r#"{"type":"assistant","requestId":"a","message":{"id":"m","model":"claude-opus-5","usage":{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":40}}}"#;
        let u = parse_transcript(line, Format::Claude);
        assert_eq!(u.tokens.cache_write_5m, 40);
        assert_eq!(u.tokens.cache_write_1h, 0);
    }

    #[test]
    fn an_unknown_model_yields_tokens_but_no_cost() {
        // "<synthetic>" appears in real transcripts, and a model released
        // after this build will look the same. Tokens are still true.
        let text =
            [rec("a", "<synthetic>", 100, 100), rec("b", "some-model-from-next-year", 100, 100)]
                .join("\n");
        let u = parse_transcript(&text, Format::Claude);
        assert_eq!(u.records, 2);
        assert_eq!(u.tokens.input, 200);
        assert_eq!(u.unpriced_records, 2);
        assert!(!u.cost_is_complete());
        assert_eq!(u.cents(), None, "no priceable record means no cost figure at all");
    }

    #[test]
    fn a_partly_priceable_total_reports_cost_but_flags_it_incomplete() {
        let text = [rec("a", "claude-opus-5", 1_000_000, 0), rec("b", "<synthetic>", 1_000_000, 0)]
            .join("\n");
        let u = parse_transcript(&text, Format::Claude);
        assert_eq!(u.cents(), Some(500), "only the priced record contributes");
        assert!(!u.cost_is_complete(), "so the UI must not call it a total");
    }

    #[test]
    fn prices_a_million_tokens_at_the_published_rate() {
        let u = parse_transcript(&rec("a", "claude-opus-5", 1_000_000, 1_000_000), Format::Claude);
        // $5 in + $25 out = $30.00
        assert_eq!(u.cents(), Some(3_000));
        assert!(u.cost_is_complete());
    }

    #[test]
    fn model_families_are_matched_by_prefix() {
        for m in ["claude-opus-5", "claude-opus-4-8", "claude-fable-5", "claude-haiku-4-5-20251001"]
        {
            assert!(model_priced(m), "{m} should be priced");
        }
        assert!(!model_priced("<synthetic>"));
        assert!(!model_priced(""));
    }

    #[test]
    fn a_single_cheap_turn_does_not_round_away_before_summing() {
        // One 100-token output on haiku is a fraction of a cent. Summed over
        // many turns it must still add up, which is why the accumulator is in
        // millicents rather than cents.
        let one = parse_transcript(&rec("a", "claude-haiku-4-5", 0, 100), Format::Claude);
        assert!(one.millicents > 0, "sub-cent cost must survive as millicents");
        assert_eq!(one.cents(), Some(0));
    }

    #[test]
    fn cache_reads_are_far_cheaper_than_input() {
        let r = rates_for("claude-opus-5").unwrap();
        assert_eq!(r.cache_read * 10, r.input);
    }

    #[test]
    fn merge_adds_every_field() {
        let mut a = parse_transcript(&rec("a", "claude-opus-5", 1, 2), Format::Claude);
        let b = parse_transcript(&rec("b", "<synthetic>", 3, 4), Format::Claude);
        a.merge(&b);
        assert_eq!(a.records, 2);
        assert_eq!(a.tokens.input, 4);
        assert_eq!(a.unpriced_records, 1);
    }

    #[test]
    fn empty_usage_offers_no_cost() {
        assert_eq!(Usage::default().cents(), None);
        assert_eq!(parse_transcript("", Format::Claude).records, 0);
    }

    #[test]
    fn cache_rereads_only_what_changed() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("s.jsonl");
        std::fs::write(&f, rec("a", "claude-opus-5", 10, 20)).unwrap();

        let mut cache = UsageCache::new();
        assert_eq!(cache.refresh(dir.path(), Format::Claude).tokens.output, 20);

        // Every mtime here is set explicitly. Writing and trusting the clock
        // to advance is flaky: two writes inside the filesystem's timestamp
        // resolution produce the same mtime, and with the length held equal
        // the stamp does not move, so the "changed" half of this test failed
        // roughly two runs in three.
        let at = |secs: u64| {
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
            std::fs::FileTimes::new().set_accessed(t).set_modified(t)
        };
        let set_mtime = |secs: u64| {
            std::fs::File::options().write(true).open(&f).unwrap().set_times(at(secs)).unwrap();
        };
        set_mtime(1_000);
        assert_eq!(cache.refresh(dir.path(), Format::Claude).tokens.output, 20);

        // Same byte length, same mtime: the stamp genuinely has not moved, so
        // the cache must serve the old figure. That is the only proof it
        // skipped the read.
        let changed = rec("a", "claude-opus-5", 10, 21);
        assert_eq!(changed.len(), rec("a", "claude-opus-5", 10, 20).len(), "same length");
        std::fs::write(&f, &changed).unwrap();
        set_mtime(1_000);
        assert_eq!(
            cache.refresh(dir.path(), Format::Claude).tokens.output,
            20,
            "unchanged stamp means no re-read"
        );

        // Move only the mtime and the new figure comes through.
        set_mtime(2_000);
        assert_eq!(cache.refresh(dir.path(), Format::Claude).tokens.output, 21);
    }

    #[test]
    fn cache_totals_across_files_and_forgets_deleted_ones() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        std::fs::write(&a, rec("a", "claude-opus-5", 10, 0)).unwrap();
        std::fs::write(&b, rec("b", "claude-opus-5", 5, 0)).unwrap();

        let mut cache = UsageCache::new();
        assert_eq!(cache.refresh(dir.path(), Format::Claude).tokens.input, 15);

        std::fs::remove_file(&b).unwrap();
        assert_eq!(
            cache.refresh(dir.path(), Format::Claude).tokens.input,
            10,
            "a cleared file stops counting"
        );
    }

    #[test]
    fn cache_ignores_non_transcript_files_and_missing_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "hello").unwrap();
        let mut cache = UsageCache::new();
        assert_eq!(cache.refresh(dir.path(), Format::Claude).records, 0);
        assert_eq!(cache.refresh(&dir.path().join("nope"), Format::Claude).records, 0);
    }

    #[test]
    fn two_sessions_in_one_worktree_both_count() {
        // Agency cuts one worktree per run, so this directory is already
        // scoped to the run: extra agent tabs share the worktree and belong to
        // the same run. Per-directory aggregation is therefore per-run
        // aggregation, and no session-id filtering is needed.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("s1.jsonl"), rec("a", "claude-opus-5", 10, 0)).unwrap();
        std::fs::write(dir.path().join("s2.jsonl"), rec("b", "claude-opus-5", 10, 0)).unwrap();
        assert_eq!(UsageCache::new().refresh(dir.path(), Format::Claude).tokens.input, 20);
    }
}
