use chrono::{Datelike, Local, NaiveDate, TimeZone};
use cosmic::iced::Subscription;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyTokens {
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub requests: u64,
}

impl DailyTokens {
    pub fn total(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    pub fn add_from(&mut self, other: &Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(other.cached_input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.requests = self.requests.saturating_add(other.requests);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageStore {
    /// Chronologically ordered map keyed by local date `"YYYY-MM-DD"`.
    #[serde(default)]
    pub days: BTreeMap<String, DailyTokens>,
    #[serde(default)]
    pub last_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LiveGeneration {
    pub prompt_tokens: u64,
    pub generated_so_far: u64,
    pub current_tok_per_sec: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct TelemetryState {
    pub store: UsageStore,
    pub last_input_tokens: u64,
    pub last_output_tokens: u64,
    pub last_tok_per_sec: Option<f64>,
    pub active_generation: Option<LiveGeneration>,
    pending_task_prompt_tokens: Option<u64>,
    pending_eval_in_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum TelemetryEvent {
    JournalJsonLine(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum IngestOutcome {
    Ignored,
    LiveProgressUpdated,
    RequestCompleted,
    ModelActivityDetected,
}

#[derive(Deserialize)]
struct JournalJsonEntry {
    #[serde(rename = "__CURSOR", default)]
    cursor: String,
    #[serde(rename = "__REALTIME_TIMESTAMP", default)]
    realtime_timestamp: String,
    #[serde(rename = "MESSAGE", default)]
    message: serde_json::Value,
}

impl TelemetryState {
    pub fn today_key() -> String {
        Local::now().format("%Y-%m-%d").to_string()
    }

    pub fn today_usage(&self) -> DailyTokens {
        let key = Self::today_key();
        self.store.days.get(&key).cloned().unwrap_or_default()
    }

    pub fn last_n_days_total(&self, days: i64) -> DailyTokens {
        let today = Local::now().date_naive();
        let cutoff = today - chrono::Duration::days(days.saturating_sub(1));
        let cutoff_str = cutoff.format("%Y-%m-%d").to_string();
        let mut sum = DailyTokens::default();
        for (date_key, entry) in self.store.days.range(cutoff_str..) {
            if date_key.as_str() <= today.format("%Y-%m-%d").to_string().as_str() {
                sum.add_from(entry);
            }
        }
        sum
    }

    pub fn all_time_total(&self) -> DailyTokens {
        let mut sum = DailyTokens::default();
        for entry in self.store.days.values() {
            sum.add_from(entry);
        }
        sum
    }

    /// Returns daily rows newest-first: the continuous recent `min_recent_days` window
    /// (`Today` down to `min_recent_days - 1` days ago), followed by any older recorded days.
    /// Tuple items: `(yyyy_mm_dd, short_label, tokens)`.
    pub fn history_bars_series(&self, min_recent_days: usize) -> Vec<(String, String, DailyTokens)> {
        let count = min_recent_days.max(1);
        let today = Local::now().date_naive();
        let mut series = Vec::with_capacity(count.max(self.store.days.len()));
        let mut oldest_recent_key = String::new();

        for offset in 0..count {
            let date = today - chrono::Duration::days(offset as i64);
            let key = date.format("%Y-%m-%d").to_string();
            let short_label = if offset == 0 {
                "Today".to_string()
            } else {
                format!("{} {:02}", month_abbrev(date.month()), date.day())
            };
            let tokens = self.store.days.get(&key).cloned().unwrap_or_default();
            oldest_recent_key = key.clone();
            series.push((key, short_label, tokens));
        }

        for (date_key, tokens) in self.store.days.range(..oldest_recent_key).rev() {
            if tokens.total() > 0 {
                series.push((
                    date_key.clone(),
                    format_short_date(date_key),
                    tokens.clone(),
                ));
            }
        }

        series
    }

    /// Ingest a single `journalctl -o json` line.
    pub fn ingest_json_line(&mut self, json_line: &str) -> IngestOutcome {
        let trimmed = json_line.trim();
        if trimmed.is_empty() {
            return IngestOutcome::Ignored;
        }
        let Ok(entry) = serde_json::from_str::<JournalJsonEntry>(trimmed) else {
            return IngestOutcome::Ignored;
        };

        let msg = match &entry.message {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(arr) => {
                let bytes: Vec<u8> = arr
                    .iter()
                    .filter_map(|v| v.as_u64().map(|b| b as u8))
                    .collect();
                String::from_utf8_lossy(&bytes).into_owned()
            }
            _ => return IngestOutcome::Ignored,
        };

        let date_str = parse_realtime_usec_to_local_date(&entry.realtime_timestamp)
            .unwrap_or_else(Self::today_key);

        let outcome = self.ingest_message(&date_str, &msg);
        if outcome == IngestOutcome::RequestCompleted && !entry.cursor.is_empty() {
            self.store.last_cursor = Some(entry.cursor);
        }
        outcome
    }

    pub fn ingest_message(&mut self, date_str: &str, msg: &str) -> IngestOutcome {
        // 1. Detect start of a prompt evaluation with full conversation token count:
        //    "slot   operator(): id  0 | task 1196 | new prompt, n_ctx_slot = 4096, n_keep = 4, task.n_tokens = 2050"
        if msg.contains("new prompt") && msg.contains("task.n_tokens =") {
            if let Some(n_tok) = extract_u64_after(msg, "task.n_tokens =") {
                self.pending_task_prompt_tokens = Some(n_tok);
                self.pending_eval_in_tokens = None;
                self.active_generation = Some(LiveGeneration {
                    prompt_tokens: n_tok,
                    generated_so_far: 0,
                    current_tok_per_sec: None,
                });
                return IngestOutcome::LiveProgressUpdated;
            }
        }

        // 2. Detect live mid-stream generation speed updates:
        //    "slot print_timing: id  0 | task 1196 | n_gen =    394, tg = 130.82 t/s, tg_3s = 131.15 t/s"
        if msg.contains("slot print_timing:") && msg.contains("n_gen =") {
            let n_gen = extract_u64_after(msg, "n_gen =").unwrap_or(0);
            let tps = extract_f64_after(msg, "tg_3s =").or_else(|| extract_f64_after(msg, "tg ="));
            let prompt_tokens = self
                .active_generation
                .as_ref()
                .map(|g| g.prompt_tokens)
                .or(self.pending_task_prompt_tokens)
                .unwrap_or(0);
            self.active_generation = Some(LiveGeneration {
                prompt_tokens,
                generated_so_far: n_gen,
                current_tok_per_sec: tps,
            });
            return IngestOutcome::LiveProgressUpdated;
        }

        // 3. Detect prompt eval timing line:
        //    "slot print_timing: id  0 | task 1196 | prompt eval time =     101.11 ms /   512 tokens (...)"
        if msg.contains("prompt eval time =") {
            if let Some(eval_in) = extract_tokens_after_slash(msg, "prompt eval time =") {
                self.pending_eval_in_tokens = Some(eval_in);
                return IngestOutcome::LiveProgressUpdated;
            }
        }

        // 4. Detect completion eval timing line (note leading spaces before `eval time =`, distinct from `prompt eval time =`):
        //    "slot print_timing: id  0 | task 1196 |        eval time =    3128.45 ms /   410 tokens (    7.65 ms per token,   130.74 tokens per second)"
        if msg.contains("eval time =") && !msg.contains("prompt eval time =") {
            if let Some(out_tokens) = extract_tokens_after_slash(msg, "eval time =") {
                let eval_in = self.pending_eval_in_tokens.take().unwrap_or(0);
                let full_in = self
                    .pending_task_prompt_tokens
                    .take()
                    .map(|t| t.max(eval_in))
                    .unwrap_or(eval_in);
                let cached_in = full_in.saturating_sub(eval_in);
                let tps = extract_tokens_per_second(msg);

                let day_bucket = self.store.days.entry(date_str.to_string()).or_default();
                day_bucket.input_tokens = day_bucket.input_tokens.saturating_add(full_in);
                day_bucket.cached_input_tokens =
                    day_bucket.cached_input_tokens.saturating_add(cached_in);
                day_bucket.output_tokens = day_bucket.output_tokens.saturating_add(out_tokens);
                day_bucket.requests = day_bucket.requests.saturating_add(1);

                self.last_input_tokens = full_in;
                self.last_output_tokens = out_tokens;
                if let Some(rate) = tps {
                    self.last_tok_per_sec = Some(rate);
                }
                self.active_generation = None;
                return IngestOutcome::RequestCompleted;
            }
        }

        // 5. Clear active generation if all slots return to idle (e.g. cancelled request)
        if msg.contains("all slots are idle") {
            if self.active_generation.is_some() {
                self.active_generation = None;
                return IngestOutcome::LiveProgressUpdated;
            }
        }

        // 6. Detect model load/unload activity to trigger immediate /api/ps refresh
        if msg.contains("starting llama-server")
            || msg.contains("runner.go")
            || (msg.contains("[GIN]") && msg.contains("POST"))
        {
            return IngestOutcome::ModelActivityDetected;
        }

        IngestOutcome::Ignored
    }
}

fn parse_realtime_usec_to_local_date(raw_usec: &str) -> Option<String> {
    let usec: i64 = raw_usec.trim().parse().ok()?;
    let secs = usec.div_euclid(1_000_000);
    let nsecs = (usec.rem_euclid(1_000_000) * 1000) as u32;
    let dt = Local.timestamp_opt(secs, nsecs).single()?;
    Some(dt.format("%Y-%m-%d").to_string())
}

fn extract_u64_after(haystack: &str, marker: &str) -> Option<u64> {
    let idx = haystack.find(marker)?;
    let rest = haystack[idx + marker.len()..].trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn extract_f64_after(haystack: &str, marker: &str) -> Option<f64> {
    let idx = haystack.find(marker)?;
    let rest = haystack[idx + marker.len()..].trim_start();
    let num_str: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num_str.parse().ok()
}

fn extract_tokens_after_slash(haystack: &str, marker: &str) -> Option<u64> {
    let idx = haystack.find(marker)?;
    let after_marker = &haystack[idx + marker.len()..];
    let slash_idx = after_marker.find('/')?;
    let after_slash = after_marker[slash_idx + 1..].trim_start();
    let digits: String = after_slash
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn extract_tokens_per_second(haystack: &str) -> Option<f64> {
    let target = "tokens per second";
    let end_idx = haystack.find(target)?;
    let before = haystack[..end_idx].trim_end();
    let start_idx = before
        .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map(|i| i + 1)
        .unwrap_or(0);
    let num_str = &before[start_idx..];
    let val: f64 = num_str.parse().ok()?;
    if val.is_finite() && val > 0.0 {
        Some(val)
    } else {
        None
    }
}

pub fn usage_store_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed)
                .join("cosmic-applet-ollama")
                .join("token_usage.json");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".local/share/cosmic-applet-ollama")
        .join("token_usage.json")
}

pub async fn save_usage_store(store: &UsageStore) -> Result<(), String> {
    let path = usage_store_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(store).map_err(|e| e.to_string())?;
    tokio::fs::write(&tmp_path, data)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp_path, &path)
        .await
        .map_err(|e| e.to_string())
}

pub async fn clear_usage_history(current_cursor: Option<String>) -> UsageStore {
    let store = UsageStore {
        days: BTreeMap::new(),
        last_cursor: current_cursor,
    };
    let _ = save_usage_store(&store).await;
    store
}

pub async fn init_telemetry() -> TelemetryState {
    let path = usage_store_path();
    let existing_store: UsageStore = match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => UsageStore::default(),
    };

    let mut state = TelemetryState {
        store: existing_store,
        ..Default::default()
    };

    // Catch up from journalctl using saved cursor if present
    let mut updated = false;
    let catchup_output = if let Some(cursor) = &state.store.last_cursor {
        let out = Command::new("journalctl")
            .args([
                "-u",
                "ollama.service",
                "--no-pager",
                "-o",
                "json",
                &format!("--after-cursor={cursor}"),
            ])
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => Some(o.stdout),
            _ => {
                // Cursor may have rotated out; catch up from latest recorded date or today
                let since = state
                    .store
                    .days
                    .keys()
                    .next_back()
                    .cloned()
                    .unwrap_or_else(TelemetryState::today_key);
                Command::new("journalctl")
                    .args([
                        "-u",
                        "ollama.service",
                        "--no-pager",
                        "-o",
                        "json",
                        "--since",
                        &since,
                    ])
                    .output()
                    .await
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| o.stdout)
            }
        }
    } else {
        Command::new("journalctl")
            .args(["-u", "ollama.service", "--no-pager", "-o", "json"])
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| o.stdout)
    };

    if let Some(stdout) = catchup_output {
        for line in String::from_utf8_lossy(&stdout).lines() {
            if state.ingest_json_line(line) == IngestOutcome::RequestCompleted {
                updated = true;
            }
        }
        // Clear any stale active_generation flag left over from historical logs
        state.active_generation = None;
    }

    if updated {
        let _ = save_usage_store(&state.store).await;
    }

    state
}

pub fn journal_subscription() -> Subscription<TelemetryEvent> {
    Subscription::run(journal_follow_stream)
}

fn journal_follow_stream() -> impl cosmic::iced::futures::Stream<Item = TelemetryEvent> {
    cosmic::iced::stream::channel(
        64,
        |mut output: cosmic::iced::futures::channel::mpsc::Sender<TelemetryEvent>| async move {
            use cosmic::iced::futures::SinkExt;

        loop {
            let child_res = Command::new("journalctl")
                .args(["-u", "ollama.service", "-f", "-n", "0", "-o", "json"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn();

            if let Ok(mut child) = child_res {
                if let Some(stdout) = child.stdout.take() {
                    let mut lines = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if output
                            .send(TelemetryEvent::JournalJsonLine(line))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                let _ = child.wait().await;
            }

            // Backoff before retrying if journalctl exited
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    })
}

pub fn format_tokens(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.2}M", count as f64 / 1_000_000.0)
    } else if count >= 10_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else if count >= 1_000 {
        let thousands = count / 1_000;
        let rem = count % 1_000;
        format!("{thousands},{rem:03}")
    } else {
        count.to_string()
    }
}

pub fn format_tokens_compact(count: u64) -> String {
    if count == 0 {
        String::new()
    } else if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

fn format_short_date(date_str: &str) -> String {
    let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
        return date_str.to_string();
    };
    let today = Local::now().date_naive();
    if date.year() == today.year() {
        format!("{} {:02}", month_abbrev(date.month()), date.day())
    } else {
        format!(
            "{} {:02} '{:02}",
            month_abbrev(date.month()),
            date.day(),
            date.year() % 100
        )
    }
}

fn month_abbrev(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ingest_multi_day_journal_lines() {
        let mut state = TelemetryState::default();

        // Day 1: 2026-10-29 (1793250000000000 usec)
        let d1_prompt = r#"{"__CURSOR":"c1","__REALTIME_TIMESTAMP":"1793250000000000","MESSAGE":"slot   operator(): id  0 | task 510 | new prompt, n_ctx_slot = 4096, n_keep = 4, task.n_tokens = 737"}"#;
        let d1_peval = r#"{"__CURSOR":"c2","__REALTIME_TIMESTAMP":"1793250001000000","MESSAGE":"slot print_timing: id  0 | task 510 | prompt eval time =     195.34 ms /   737 tokens (    0.27 ms per token,  3772.87 tokens per second)"}"#;
        let d1_eval = r#"{"__CURSOR":"c3","__REALTIME_TIMESTAMP":"1793250005000000","MESSAGE":"slot print_timing: id  0 | task 510 |        eval time =    4851.49 ms /   683 tokens (    7.11 ms per token,   140.58 tokens per second)"}"#;

        assert_eq!(state.ingest_json_line(d1_prompt), IngestOutcome::LiveProgressUpdated);
        assert_eq!(state.ingest_json_line(d1_peval), IngestOutcome::LiveProgressUpdated);
        assert_eq!(state.ingest_json_line(d1_eval), IngestOutcome::RequestCompleted);
        assert_eq!(state.store.last_cursor.as_deref(), Some("c3"));

        // Day 2: 2026-10-30 (+86400s -> 1793336400000000 usec), with KV cache hit (2050 full prompt, 512 newly evaluated = 1538 cached)
        let d2_prompt = r#"{"__CURSOR":"c4","__REALTIME_TIMESTAMP":"1793336400000000","MESSAGE":"slot   operator(): id  0 | task 1196 | new prompt, n_ctx_slot = 4096, n_keep = 4, task.n_tokens = 2050"}"#;
        let d2_live = r#"{"__CURSOR":"c5","__REALTIME_TIMESTAMP":"1793336403000000","MESSAGE":"slot print_timing: id  0 | task 1196 | n_gen =    394, tg = 130.82 t/s, tg_3s = 131.15 t/s"}"#;
        let d2_peval = r#"{"__CURSOR":"c6","__REALTIME_TIMESTAMP":"1793336404000000","MESSAGE":"slot print_timing: id  0 | task 1196 | prompt eval time =     101.11 ms /   512 tokens (    0.20 ms per token,  5063.54 tokens per second)"}"#;
        let d2_eval = r#"{"__CURSOR":"c7","__REALTIME_TIMESTAMP":"1793336404500000","MESSAGE":"slot print_timing: id  0 | task 1196 |        eval time =    3128.45 ms /   410 tokens (    7.65 ms per token,   130.74 tokens per second)"}"#;

        state.ingest_json_line(d2_prompt);
        state.ingest_json_line(d2_live);
        assert_eq!(
            state.active_generation.as_ref().unwrap().generated_so_far,
            394
        );
        state.ingest_json_line(d2_peval);
        assert_eq!(state.ingest_json_line(d2_eval), IngestOutcome::RequestCompleted);

        assert!(state.active_generation.is_none());
        assert_eq!(state.store.last_cursor.as_deref(), Some("c7"));
        assert_eq!(state.store.days.len(), 2);

        let all = state.all_time_total();
        assert_eq!(all.input_tokens, 737 + 2050);
        assert_eq!(all.cached_input_tokens, 1538);
        assert_eq!(all.output_tokens, 683 + 410);
        assert_eq!(all.requests, 2);
        assert!((state.last_tok_per_sec.unwrap() - 130.74).abs() < 0.01);
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(410), "410");
        assert_eq!(format_tokens(2_787), "2,787");
        assert_eq!(format_tokens(18_420), "18.4K");
        assert_eq!(format_tokens(1_250_000), "1.25M");
        assert_eq!(format_tokens_compact(0), "");
        assert_eq!(format_tokens_compact(850), "850");
        assert_eq!(format_tokens_compact(2_787), "2.8K");
        assert_eq!(format_tokens_compact(27_002), "27.0K");
    }

    #[test]
    fn test_history_bars_series() {
        let mut state = TelemetryState::default();
        let today_key = TelemetryState::today_key();
        state.store.days.insert(
            today_key.clone(),
            DailyTokens {
                input_tokens: 1200,
                cached_input_tokens: 200,
                output_tokens: 600,
                requests: 3,
            },
        );
        state.store.days.insert(
            "2026-01-05".to_string(),
            DailyTokens {
                input_tokens: 500,
                cached_input_tokens: 100,
                output_tokens: 250,
                requests: 1,
            },
        );
        let series = state.history_bars_series(7);
        assert_eq!(series.len(), 8);
        assert_eq!(series[0].0, today_key);
        assert_eq!(series[0].1, "Today");
        assert_eq!(series[0].2.total(), 1800);
        assert_eq!(series[7].0, "2026-01-05");
        assert_eq!(series[7].2.total(), 750);
    }
}
