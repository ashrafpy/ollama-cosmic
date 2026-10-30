use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tokio::process::Command;

static PARAMS_CACHE: LazyLock<Mutex<HashMap<(String, String), ModelParams>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelParams {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<i32>,
    pub repeat_penalty: Option<f32>,
    pub num_ctx: Option<u64>,
    pub num_predict: Option<i32>,
    /// Preserves untouched Modelfile parameters (e.g. `stop` tokens, `repeat_last_n`, `min_p`).
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ModelParams {
    pub fn summary_line(&self) -> String {
        let mut parts = Vec::new();
        if let Some(t) = self.temperature {
            parts.push(format!("temp {t:.2}"));
        }
        if let Some(tp) = self.top_p {
            parts.push(format!("top_p {tp:.2}"));
        }
        if let Some(tk) = self.top_k {
            parts.push(format!("top_k {tk}"));
        }
        if let Some(rp) = self.repeat_penalty {
            parts.push(format!("rep {rp:.2}"));
        }
        if let Some(ctx) = self.num_ctx {
            parts.push(format!("ctx {}", format_context(ctx)));
        }
        if let Some(np) = self.num_predict {
            if np > 0 {
                parts.push(format!("max_out {np}"));
            }
        }
        if parts.is_empty() {
            "Model defaults".to_string()
        } else {
            parts.join(" · ")
        }
    }

    pub fn to_json_parameters(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = self.extra.clone();
        if let Some(t) = self.temperature {
            // Round to 2 decimal places cleanly to avoid f32 representation noise like 0.70000005
            let rounded = (f64::from(t) * 100.0).round() / 100.0;
            if let Some(n) = serde_json::Number::from_f64(rounded) {
                map.insert("temperature".to_string(), serde_json::Value::Number(n));
            }
        }
        if let Some(tp) = self.top_p {
            let rounded = (f64::from(tp) * 100.0).round() / 100.0;
            if let Some(n) = serde_json::Number::from_f64(rounded) {
                map.insert("top_p".to_string(), serde_json::Value::Number(n));
            }
        }
        if let Some(tk) = self.top_k {
            map.insert("top_k".to_string(), serde_json::Value::from(tk));
        }
        if let Some(rp) = self.repeat_penalty {
            let rounded = (f64::from(rp) * 100.0).round() / 100.0;
            if let Some(n) = serde_json::Number::from_f64(rounded) {
                map.insert("repeat_penalty".to_string(), serde_json::Value::Number(n));
            }
        }
        if let Some(ctx) = self.num_ctx {
            map.insert("num_ctx".to_string(), serde_json::Value::from(ctx));
        } else {
            map.remove("num_ctx");
        }
        if let Some(np) = self.num_predict {
            map.insert("num_predict".to_string(), serde_json::Value::from(np));
        } else {
            map.remove("num_predict");
        }
        map
    }
}

pub fn parse_parameters_str(raw: &str) -> ModelParams {
    let mut params = ModelParams::default();
    let mut stops: Vec<serde_json::Value> = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, val_raw)) = split_first_whitespace(trimmed) else {
            continue;
        };
        let unquoted = strip_surrounding_quotes(val_raw.trim());

        match key {
            "temperature" => {
                if let Ok(v) = unquoted.parse::<f32>() {
                    params.temperature = Some(v);
                }
            }
            "top_p" => {
                if let Ok(v) = unquoted.parse::<f32>() {
                    params.top_p = Some(v);
                }
            }
            "top_k" => {
                if let Ok(v) = unquoted.parse::<i32>() {
                    params.top_k = Some(v);
                }
            }
            "repeat_penalty" => {
                if let Ok(v) = unquoted.parse::<f32>() {
                    params.repeat_penalty = Some(v);
                }
            }
            "num_ctx" => {
                if let Ok(v) = unquoted.parse::<u64>() {
                    params.num_ctx = Some(v);
                }
            }
            "num_predict" => {
                if let Ok(v) = unquoted.parse::<i32>() {
                    params.num_predict = Some(v);
                }
            }
            "stop" => {
                stops.push(serde_json::Value::String(unquoted.to_string()));
            }
            other => {
                let json_val = if let Ok(i) = unquoted.parse::<i64>() {
                    serde_json::Value::from(i)
                } else if let Ok(f) = unquoted.parse::<f64>() {
                    serde_json::Number::from_f64(f)
                        .map(serde_json::Value::Number)
                        .unwrap_or_else(|| serde_json::Value::String(unquoted.to_string()))
                } else if unquoted == "true" {
                    serde_json::Value::Bool(true)
                } else if unquoted == "false" {
                    serde_json::Value::Bool(false)
                } else {
                    serde_json::Value::String(unquoted.to_string())
                };
                params.extra.insert(other.to_string(), json_val);
            }
        }
    }

    if !stops.is_empty() {
        params
            .extra
            .insert("stop".to_string(), serde_json::Value::Array(stops));
    }

    params
}

fn split_first_whitespace(s: &str) -> Option<(&str, &str)> {
    let idx = s.find(char::is_whitespace)?;
    Some((&s[..idx], s[idx..].trim_start()))
}

fn strip_surrounding_quotes(s: &str) -> &str {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModelDetails {
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub parameter_size: String,
    #[serde(default)]
    pub quantization_level: String,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub runner: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PsResponse {
    #[serde(default)]
    pub models: Vec<PsModelEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PsModelEntry {
    pub name: String,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub size_vram: u64,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub details: ModelDetails,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TagsResponse {
    #[serde(default)]
    pub models: Vec<TagModelEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TagModelEntry {
    pub name: String,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub details: ModelDetails,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct VersionResponse {
    #[serde(default)]
    version: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ShowResponse {
    #[serde(default)]
    parameters: String,
}

#[derive(Serialize)]
struct ShowRequest<'a> {
    model: &'a str,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RunningModel {
    pub name: String,
    pub display_name: String,
    pub size_bytes: u64,
    pub size_vram_bytes: u64,
    pub expires_at: Option<DateTime<Utc>>,
    pub context_length: Option<u64>,
    pub max_context_length: Option<u64>,
    pub parameter_size: String,
    pub quantization: String,
    pub family: String,
    pub runner: String,
    pub capabilities: Vec<String>,
    pub params: ModelParams,
}

#[allow(dead_code)]
impl RunningModel {
    /// Processor placement summary (e.g. "100% GPU", "68% GPU · 32% CPU", "100% CPU")
    pub fn processor_summary(&self) -> String {
        if self.size_bytes == 0 {
            return "Unknown".to_string();
        }
        if self.size_vram_bytes == 0 {
            "100% CPU".to_string()
        } else if self.size_vram_bytes >= self.size_bytes {
            "100% GPU".to_string()
        } else {
            let gpu_pct =
                ((self.size_vram_bytes as f64 / self.size_bytes as f64) * 100.0).round() as u64;
            let cpu_pct = 100_u64.saturating_sub(gpu_pct);
            format!("{gpu_pct}% GPU · {cpu_pct}% CPU")
        }
    }

    pub fn is_pinned_forever(&self, now: DateTime<Utc>) -> bool {
        match self.expires_at {
            None => true,
            Some(exp) => (exp - now).num_seconds() > 86_400 * 365,
        }
    }

    /// Time remaining before automatic keep_alive unload
    pub fn expiry_summary(&self, now: DateTime<Utc>) -> String {
        let Some(exp) = self.expires_at else {
            return "Forever".to_string();
        };
        let secs = (exp - now).num_seconds();
        if secs <= 0 {
            return "Unloading…".to_string();
        }
        if secs > 86_400 * 365 {
            return "Forever".to_string();
        }
        if secs >= 3600 {
            let h = secs / 3600;
            let m = (secs % 3600) / 60;
            format!("{h}h {m}m")
        } else if secs >= 60 {
            let m = secs / 60;
            let s = secs % 60;
            format!("{m}m {s:02}s")
        } else {
            format!("{secs}s")
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstalledModel {
    pub name: String,
    pub display_name: String,
    pub digest: String,
    pub size_bytes: u64,
    pub parameter_size: String,
    pub quantization: String,
    pub family: String,
    pub runner: String,
    pub context_length: Option<u64>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ServiceState {
    Online { version: String },
    Offline { systemd_state: String, error: String },
}

#[derive(Debug, Clone)]
pub struct OllamaSnapshot {
    pub host: String,
    pub lan_endpoint: Option<String>,
    pub lan_exposed: bool,
    pub state: ServiceState,
    pub running: Vec<RunningModel>,
    pub installed: Vec<InstalledModel>,
}

impl Default for OllamaSnapshot {
    fn default() -> Self {
        let host = resolve_ollama_host();
        let (lan_endpoint, lan_exposed) = resolve_lan_endpoint(&host);
        Self {
            host,
            lan_endpoint,
            lan_exposed,
            state: ServiceState::Offline {
                systemd_state: "checking…".to_string(),
                error: "Connecting to Ollama…".to_string(),
            },
            running: Vec::new(),
            installed: Vec::new(),
        }
    }
}

pub fn resolve_ollama_host() -> String {
    let raw = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return "http://127.0.0.1:11434".to_string();
    }
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    with_scheme.replace("://0.0.0.0", "://127.0.0.1")
}

pub fn extract_port(api_host: &str) -> Option<u16> {
    let without_scheme = api_host
        .trim()
        .strip_prefix("http://")
        .or_else(|| api_host.trim().strip_prefix("https://"))
        .unwrap_or(api_host.trim());
    let authority = without_scheme.split('/').next()?;
    let port_str = if let Some(rest) = authority.strip_prefix('[') {
        let (_, after_bracket) = rest.split_once(']')?;
        after_bracket.strip_prefix(':')?
    } else {
        let (_, port_part) = authority.rsplit_once(':')?;
        port_part
    };
    port_str.parse::<u16>().ok()
}

fn is_loopback_proc_ip(ip_hex: &str) -> bool {
    let upper = ip_hex.to_ascii_uppercase();
    if upper.len() == 8 {
        if upper == "0100007F" || upper.ends_with("7F") {
            return true;
        }
        if let Ok(raw) = u32::from_str_radix(&upper, 16) {
            return std::net::Ipv4Addr::from(raw.to_ne_bytes()).is_loopback();
        }
        false
    } else if upper.len() == 32 {
        upper == "00000000000000000000000001000000"
            || upper == "00000000000000000000000000000001"
            || (upper.starts_with("0000000000000000FFFF0000") && upper.ends_with("7F"))
    } else {
        false
    }
}

pub fn parse_proc_net_tcp_exposed(content: &str, port: u16) -> bool {
    let port_hex = format!("{port:04X}");
    for line in content.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let _sl = cols.next();
        let Some(local_addr) = cols.next() else {
            continue;
        };
        let _rem_addr = cols.next();
        let Some(state) = cols.next() else {
            continue;
        };
        if !state.eq_ignore_ascii_case("0A") {
            continue;
        }
        let Some((ip_hex, lport_hex)) = local_addr.split_once(':') else {
            continue;
        };
        if lport_hex.eq_ignore_ascii_case(&port_hex) && !is_loopback_proc_ip(ip_hex) {
            return true;
        }
    }
    false
}

pub fn is_port_exposed_on_lan(port: u16) -> bool {
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(content) = std::fs::read_to_string(path) {
            if parse_proc_net_tcp_exposed(&content, port) {
                return true;
            }
        }
    }
    false
}

fn is_wireless_iface(iface: &str) -> bool {
    iface.starts_with("wl")
        || std::path::Path::new(&format!("/sys/class/net/{iface}/wireless")).exists()
        || std::path::Path::new(&format!("/sys/class/net/{iface}/phy80211")).exists()
}

fn is_virtual_or_loopback_iface(iface: &str) -> bool {
    iface == "lo"
        || iface.starts_with("lo")
        || iface.starts_with("docker")
        || iface.starts_with("br-")
        || iface.starts_with("veth")
        || iface.starts_with("virbr")
}

fn probe_local_ipv4_for_target(target: std::net::Ipv4Addr) -> Option<std::net::Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect((target, 80)).ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

fn probe_route_line_ipv4(dest_hex: &str, gateway_hex: &str, mask_hex: &str) -> Option<std::net::Ipv4Addr> {
    let dest_raw = u32::from_str_radix(dest_hex, 16).ok()?;
    let gateway_raw = u32::from_str_radix(gateway_hex, 16).ok()?;
    let mask_raw = u32::from_str_radix(mask_hex, 16).ok()?;

    if dest_raw != 0 && mask_raw != 0 && mask_raw != 0xFFFF_FFFF {
        let net = u32::from_be_bytes(dest_raw.to_ne_bytes())
            & u32::from_be_bytes(mask_raw.to_ne_bytes());
        for host_offset in [1_u32, 2_u32] {
            let target = std::net::Ipv4Addr::from(net | host_offset);
            if let Some(ip) = probe_local_ipv4_for_target(target) {
                return Some(ip);
            }
        }
    }
    if gateway_raw != 0 {
        let gw = std::net::Ipv4Addr::from(gateway_raw.to_ne_bytes());
        if let Some(ip) = probe_local_ipv4_for_target(gw) {
            return Some(ip);
        }
    }
    None
}

pub fn detect_wlan_or_lan_ipv4() -> Option<std::net::Ipv4Addr> {
    let route_table = std::fs::read_to_string("/proc/net/route").ok();

    // 1. Prefer wireless interfaces in /proc/net/route
    if let Some(ref content) = route_table {
        for line in content.lines().skip(1) {
            let mut cols = line.split_whitespace();
            let (Some(iface), Some(dest), Some(gateway), _, _, _, _, Some(mask)) = (
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
            ) else {
                continue;
            };
            if is_wireless_iface(iface) {
                if let Some(ip) = probe_route_line_ipv4(dest, gateway, mask) {
                    return Some(ip);
                }
            }
        }
    }

    // 2. Standard default route lookup via UDP socket connect
    if let Some(ip) = probe_local_ipv4_for_target(std::net::Ipv4Addr::new(1, 1, 1, 1))
        .or_else(|| probe_local_ipv4_for_target(std::net::Ipv4Addr::new(8, 8, 8, 8)))
    {
        return Some(ip);
    }

    // 3. Fallback to any non-loopback, non-virtual interface route in /proc/net/route
    if let Some(ref content) = route_table {
        for line in content.lines().skip(1) {
            let mut cols = line.split_whitespace();
            let (Some(iface), Some(dest), Some(gateway), _, _, _, _, Some(mask)) = (
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
            ) else {
                continue;
            };
            if !is_virtual_or_loopback_iface(iface) {
                if let Some(ip) = probe_route_line_ipv4(dest, gateway, mask) {
                    return Some(ip);
                }
            }
        }
    }

    None
}

pub fn resolve_lan_endpoint(api_host: &str) -> (Option<String>, bool) {
    let port = extract_port(api_host).unwrap_or(11434);
    let exposed = is_port_exposed_on_lan(port);
    let lan_ip = detect_wlan_or_lan_ipv4();
    (lan_ip.map(|ip| format!("http://{ip}:{port}")), exposed)
}

pub fn compact_model_name(name: &str) -> String {
    name.strip_suffix(":latest").unwrap_or(name).to_string()
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GiB", b / GB)
    } else if b >= MB {
        format!("{:.0} MiB", b / MB)
    } else if b >= KB {
        format!("{:.0} KiB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn format_context(tokens: u64) -> String {
    if tokens >= 1_048_576 && tokens % 1_048_576 == 0 {
        format!("{}M", tokens / 1_048_576)
    } else if tokens >= 1024 && tokens % 1024 == 0 {
        format!("{}K", tokens / 1024)
    } else if tokens >= 1000 {
        format!("{:.1}K", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}

fn build_client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_millis(1500))
        .build()
}

async fn check_systemd_unit() -> String {
    match Command::new("systemctl")
        .args(["is-active", "ollama.service"])
        .output()
        .await
    {
        Ok(out) => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() {
                "unknown".to_string()
            } else {
                s
            }
        }
        Err(_) => "unknown".to_string(),
    }
}

async fn get_cached_or_fetch_params(client: &Client, host: &str, name: &str, digest: &str) -> ModelParams {
    let cache_key = (name.to_string(), digest.to_string());
    if !digest.is_empty() {
        if let Ok(guard) = PARAMS_CACHE.lock() {
            if let Some(cached) = guard.get(&cache_key) {
                return cached.clone();
            }
        }
    }

    let show_url = format!("{host}/api/show");
    let fetched = match client
        .post(&show_url)
        .json(&ShowRequest { model: name })
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r
            .json::<ShowResponse>()
            .await
            .map(|sr| parse_parameters_str(&sr.parameters))
            .unwrap_or_default(),
        _ => ModelParams::default(),
    };

    if !digest.is_empty() {
        if let Ok(mut guard) = PARAMS_CACHE.lock() {
            guard.insert(cache_key, fetched.clone());
        }
    }
    fetched
}

pub async fn fetch_model_params(model: String) -> Result<(String, ModelParams), String> {
    let host = resolve_ollama_host();
    let client = build_client().map_err(|e| e.to_string())?;
    let show_url = format!("{host}/api/show");
    let res = client
        .post(&show_url)
        .json(&ShowRequest { model: &model })
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !res.status().is_success() {
        return Err(format!("Failed to inspect {model}: HTTP {}", res.status()));
    }
    let show: ShowResponse = res.json().await.map_err(|e| e.to_string())?;
    Ok((model, parse_parameters_str(&show.parameters)))
}

async fn build_running_models(
    client: &Client,
    host: &str,
    ps_entries: Vec<PsModelEntry>,
    installed: &[InstalledModel],
) -> Vec<RunningModel> {
    let mut inst_by_name: HashMap<&str, &InstalledModel> = HashMap::new();
    for inst in installed {
        inst_by_name.insert(inst.name.as_str(), inst);
    }

    let mut running = Vec::with_capacity(ps_entries.len());
    for m in ps_entries {
        let tag_meta = inst_by_name.get(m.name.as_str()).copied();
        let expires_at = m
            .expires_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let parameter_size = if !m.details.parameter_size.is_empty() {
            m.details.parameter_size
        } else {
            tag_meta
                .map(|t| t.parameter_size.clone())
                .unwrap_or_default()
        };

        let quantization = if !m.details.quantization_level.is_empty() {
            m.details.quantization_level
        } else {
            tag_meta.map(|t| t.quantization.clone()).unwrap_or_default()
        };

        let family = if !m.details.family.is_empty() {
            m.details.family
        } else {
            tag_meta.map(|t| t.family.clone()).unwrap_or_default()
        };

        let runner = if !m.details.runner.is_empty() {
            m.details.runner
        } else {
            tag_meta.map(|t| t.runner.clone()).unwrap_or_default()
        };

        let digest = if !m.digest.is_empty() {
            m.digest
        } else {
            tag_meta.map(|t| t.digest.clone()).unwrap_or_default()
        };

        let active_ctx = m.context_length.or(m.details.context_length);
        let max_ctx = tag_meta.and_then(|t| t.context_length);
        let capabilities = tag_meta
            .map(|t| t.capabilities.clone())
            .unwrap_or_default();

        let params = get_cached_or_fetch_params(client, host, &m.name, &digest).await;

        running.push(RunningModel {
            display_name: compact_model_name(&m.name),
            name: m.name,
            size_bytes: m.size,
            size_vram_bytes: m.size_vram,
            expires_at,
            context_length: active_ctx,
            max_context_length: max_ctx,
            parameter_size,
            quantization,
            family,
            runner,
            capabilities,
            params,
        });
    }
    running
}

/// Lightweight background poll hitting only `GET /api/ps` when already online.
pub async fn fetch_ps_only(prev: OllamaSnapshot) -> OllamaSnapshot {
    if matches!(prev.state, ServiceState::Offline { .. }) || prev.installed.is_empty() {
        return fetch_snapshot().await;
    }

    let host = resolve_ollama_host();
    let Ok(client) = build_client() else {
        return fetch_snapshot().await;
    };

    let ps_url = format!("{host}/api/ps");
    let Ok(res) = client.get(&ps_url).send().await else {
        return fetch_snapshot().await;
    };
    if !res.status().is_success() {
        return fetch_snapshot().await;
    }
    let Ok(ps_resp) = res.json::<PsResponse>().await else {
        return fetch_snapshot().await;
    };

    // If a newly loaded model isn't in `prev.installed`, perform full refresh to get its tags
    let known_names: std::collections::HashSet<&str> =
        prev.installed.iter().map(|i| i.name.as_str()).collect();
    if ps_resp
        .models
        .iter()
        .any(|m| !known_names.contains(m.name.as_str()))
    {
        return fetch_snapshot().await;
    }

    let (lan_endpoint, lan_exposed) = resolve_lan_endpoint(&host);
    let running = build_running_models(&client, &host, ps_resp.models, &prev.installed).await;
    OllamaSnapshot {
        host,
        lan_endpoint,
        lan_exposed,
        state: prev.state,
        running,
        installed: prev.installed,
    }
}

pub async fn fetch_snapshot() -> OllamaSnapshot {
    let host = resolve_ollama_host();
    let (lan_endpoint, lan_exposed) = resolve_lan_endpoint(&host);
    let client = match build_client() {
        Ok(c) => c,
        Err(e) => {
            return OllamaSnapshot {
                host,
                lan_endpoint,
                lan_exposed,
                state: ServiceState::Offline {
                    systemd_state: "unknown".to_string(),
                    error: e.to_string(),
                },
                running: Vec::new(),
                installed: Vec::new(),
            };
        }
    };

    let ps_url = format!("{host}/api/ps");
    let tags_url = format!("{host}/api/tags");
    let ver_url = format!("{host}/api/version");

    let (ps_res, tags_res, ver_res) = tokio::join!(
        client.get(&ps_url).send(),
        client.get(&tags_url).send(),
        client.get(&ver_url).send(),
    );

    let ps_resp = match ps_res {
        Ok(r) if r.status().is_success() => match r.json::<PsResponse>().await {
            Ok(parsed) => parsed,
            Err(e) => {
                let systemd_state = check_systemd_unit().await;
                return OllamaSnapshot {
                    host,
                    lan_endpoint,
                    lan_exposed,
                    state: ServiceState::Offline {
                        systemd_state,
                        error: format!("Invalid /api/ps JSON: {e}"),
                    },
                    running: Vec::new(),
                    installed: Vec::new(),
                };
            }
        },
        Ok(r) => {
            let status = r.status();
            let systemd_state = check_systemd_unit().await;
            return OllamaSnapshot {
                host,
                lan_endpoint,
                lan_exposed,
                state: ServiceState::Offline {
                    systemd_state,
                    error: format!("HTTP {status}"),
                },
                running: Vec::new(),
                installed: Vec::new(),
            };
        }
        Err(e) => {
            let systemd_state = check_systemd_unit().await;
            let short_err = if e.is_connect() {
                "Connection refused".to_string()
            } else if e.is_timeout() {
                "Connection timed out".to_string()
            } else {
                e.to_string()
            };
            return OllamaSnapshot {
                host,
                lan_endpoint,
                lan_exposed,
                state: ServiceState::Offline {
                    systemd_state,
                    error: short_err,
                },
                running: Vec::new(),
                installed: Vec::new(),
            };
        }
    };

    let version = match ver_res {
        Ok(r) if r.status().is_success() => r
            .json::<VersionResponse>()
            .await
            .map(|v| v.version)
            .unwrap_or_default(),
        _ => String::new(),
    };

    let tags_entries = match tags_res {
        Ok(r) if r.status().is_success() => r
            .json::<TagsResponse>()
            .await
            .map(|t| t.models)
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    let installed: Vec<InstalledModel> = tags_entries
        .into_iter()
        .map(|t| InstalledModel {
            display_name: compact_model_name(&t.name),
            name: t.name,
            digest: t.digest,
            size_bytes: t.size,
            parameter_size: t.details.parameter_size,
            quantization: t.details.quantization_level,
            family: t.details.family,
            runner: t.details.runner,
            context_length: t.details.context_length,
            capabilities: t
                .capabilities
                .into_iter()
                .filter(|c| c.as_str() != "completion")
                .collect(),
        })
        .collect();

    let running = build_running_models(&client, &host, ps_resp.models, &installed).await;

    OllamaSnapshot {
        host,
        lan_endpoint,
        lan_exposed,
        state: ServiceState::Online { version },
        running,
        installed,
    }
}

#[derive(Serialize)]
struct KeepAliveGenerateRequest<'a> {
    model: &'a str,
    keep_alive: serde_json::Value,
}

#[derive(Serialize)]
struct KeepAliveEmbedRequest<'a> {
    model: &'a str,
    input: [&'a str; 0],
    keep_alive: serde_json::Value,
}

pub async fn set_keep_alive(model: String, keep_alive: serde_json::Value) -> Result<String, String> {
    let host = resolve_ollama_host();
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let gen_url = format!("{host}/api/generate");
    let res = client
        .post(&gen_url)
        .json(&KeepAliveGenerateRequest {
            model: &model,
            keep_alive: keep_alive.clone(),
        })
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if res.status().is_success() {
        return Ok(model);
    }

    let embed_url = format!("{host}/api/embed");
    let res_embed = client
        .post(&embed_url)
        .json(&KeepAliveEmbedRequest {
            model: &model,
            input: [],
            keep_alive,
        })
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if res_embed.status().is_success() {
        Ok(model)
    } else {
        Err(format!(
            "Failed to update residency for {model}: HTTP {}",
            res.status()
        ))
    }
}

pub async fn unload_model(model: String) -> Result<String, String> {
    set_keep_alive(model, serde_json::Value::from(0)).await
}

#[derive(Serialize)]
struct WarmLoadRequest<'a> {
    model: &'a str,
}

pub async fn load_model(model: String) -> Result<String, String> {
    let host = resolve_ollama_host();
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let gen_url = format!("{host}/api/generate");
    let res = client
        .post(&gen_url)
        .json(&WarmLoadRequest { model: &model })
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if res.status().is_success() {
        return Ok(model);
    }

    let embed_url = format!("{host}/api/embed");
    let res_embed = client
        .post(&embed_url)
        .json(&KeepAliveEmbedRequest {
            model: &model,
            input: [],
            keep_alive: serde_json::Value::from("5m"),
        })
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if res_embed.status().is_success() {
        Ok(model)
    } else {
        Err(format!("Failed to load {model}: HTTP {}", res.status()))
    }
}

#[derive(Serialize)]
struct CreateModelRequest<'a> {
    model: &'a str,
    from: &'a str,
    parameters: serde_json::Map<String, serde_json::Value>,
    stream: bool,
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: String,
}

pub async fn save_model_params(
    source_model: String,
    target_model: String,
    params: ModelParams,
    was_running: bool,
) -> Result<String, String> {
    let host = resolve_ollama_host();
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;

    let create_url = format!("{host}/api/create");
    let res = client
        .post(&create_url)
        .json(&CreateModelRequest {
            model: &target_model,
            from: &source_model,
            parameters: params.to_json_parameters(),
            stream: false,
        })
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !res.status().is_success() {
        let status = res.status();
        let detail = res
            .json::<ErrorBody>()
            .await
            .map(|b| b.error)
            .unwrap_or_default();
        if detail.is_empty() {
            return Err(format!("Failed to save {target_model}: HTTP {status}"));
        } else {
            return Err(format!("Failed to save {target_model}: {detail}"));
        }
    }

    // Evict cached parameter entries for target_model so fresh digest/params are read
    if let Ok(mut guard) = PARAMS_CACHE.lock() {
        guard.retain(|(name, _), _| name != &target_model);
    }

    // If updated in-place while currently loaded in VRAM, reload so new num_ctx/params take effect
    if target_model == source_model && was_running {
        let _ = unload_model(target_model.clone()).await;
        let _ = load_model(target_model.clone()).await;
    }

    Ok(target_model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compact_model_name() {
        assert_eq!(compact_model_name("lfm3:latest"), "lfm3");
        assert_eq!(compact_model_name("llama3.2:3b"), "llama3.2:3b");
        assert_eq!(compact_model_name("qwen3"), "qwen3");
    }

    #[test]
    fn test_format_helpers() {
        assert_eq!(format_bytes(1_099_638_680), "1.02 GiB");
        assert_eq!(format_bytes(524_288_000), "500 MiB");
        assert_eq!(format_context(262_144), "256K");
        assert_eq!(format_context(8192), "8K");
        assert_eq!(format_context(1_048_576), "1M");
    }

    #[test]
    fn test_parse_and_roundtrip_parameters() {
        let raw = "repeat_last_n                  -1\nrepeat_penalty                 1.1\ntemperature                    0.1\ntop_k                          50\ntop_p                          1\nstop                           \"<|im_end|>\"\nstop                           \"<|eot_id|>\"";
        let mut parsed = parse_parameters_str(raw);
        assert_eq!(parsed.temperature, Some(0.1));
        assert_eq!(parsed.top_k, Some(50));
        assert_eq!(parsed.top_p, Some(1.0));
        assert_eq!(parsed.repeat_penalty, Some(1.1));
        assert_eq!(
            parsed.summary_line(),
            "temp 0.10 · top_p 1.00 · top_k 50 · rep 1.10"
        );

        // Modify temperature and num_ctx, verify untouched stop array & repeat_last_n stay intact
        parsed.temperature = Some(0.7);
        parsed.num_ctx = Some(16_384);
        let json_map = parsed.to_json_parameters();

        assert_eq!(json_map.get("temperature").unwrap().as_f64().unwrap(), 0.7);
        assert_eq!(json_map.get("num_ctx").unwrap().as_u64().unwrap(), 16_384);
        assert_eq!(json_map.get("repeat_last_n").unwrap().as_i64().unwrap(), -1);
        let stops = json_map.get("stop").unwrap().as_array().unwrap();
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].as_str().unwrap(), "<|im_end|>");
    }

    #[test]
    fn test_deserialize_tags_and_ps() {
        let tags_json = r#"{"models":[{"name":"lfm3:latest","digest":"sha256:abc","size":1099638680,"details":{"family":"lfm3","parameter_size":"1.7B","quantization_level":"Q4_K_M","context_length":262144,"runner":"llamacpp"},"capabilities":["completion","tools","thinking"]}]}"#;
        let tags: TagsResponse = serde_json::from_str(tags_json).unwrap();
        assert_eq!(tags.models.len(), 1);
        assert_eq!(tags.models[0].details.parameter_size, "1.7B");
        assert_eq!(tags.models[0].details.context_length, Some(262_144));
        assert_eq!(tags.models[0].capabilities.len(), 3);

        let ps_json = r#"{"models":[{"name":"lfm3:latest","digest":"sha256:abc","size":1500000000,"size_vram":1500000000,"expires_at":"2026-10-30T05:10:00Z","context_length":4096,"details":{"family":"lfm3","parameter_size":"1.7B","quantization_level":"Q4_K_M"}}]}"#;
        let ps: PsResponse = serde_json::from_str(ps_json).unwrap();
        assert_eq!(ps.models.len(), 1);
        assert_eq!(ps.models[0].size_vram, 1_500_000_000);
        assert_eq!(ps.models[0].context_length, Some(4096));
    }

    #[test]
    fn test_extract_port_and_lan_exposure() {
        assert_eq!(extract_port("http://127.0.0.1:11434"), Some(11434));
        assert_eq!(extract_port("https://192.168.1.50:8080/"), Some(8080));
        assert_eq!(extract_port("http://[::1]:11435"), Some(11435));
        assert_eq!(extract_port("http://localhost"), None);

        // 11434 = 0x2CAA
        let loopback_v4 = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:2CAA 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000000000000000 100 0 0 10 0\n";
        assert!(!parse_proc_net_tcp_exposed(loopback_v4, 11434));

        let wildcard_v4 = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000:2CAA 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000000000000000 100 0 0 10 0\n";
        assert!(parse_proc_net_tcp_exposed(wildcard_v4, 11434));

        let loopback_v6 = "\
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:2CAA 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000000000000000 100 0 0 10 0\n";
        assert!(!parse_proc_net_tcp_exposed(loopback_v6, 11434));

        let wildcard_v6 = "\
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:2CAA 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000000000000000 100 0 0 10 0\n";
        assert!(parse_proc_net_tcp_exposed(wildcard_v6, 11434));
    }
}
