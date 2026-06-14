use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::settings::storage::{read_json_or_default, save_pretty_json};
use crate::settings::{LlmProfile, claudie_home};

use super::config::ProxyOptimizationConfig;
use super::{OPTIMIZER_VERSION, now_millis};

const MILLIS_PER_HOUR: u128 = 60 * 60 * 1000;
const PRUNE_THROTTLE_MS: u64 = 60_000;

static LAST_CHUNK_PRUNE_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct SummaryCacheFile {
    #[serde(default)]
    pub(super) version: String,
    #[serde(default)]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) summary: String,
    #[serde(default)]
    pub(super) created_at_ms: u128,
    #[serde(default)]
    pub(super) last_used_at_ms: u128,
}

pub(super) fn load_chunk_summary(path: &Path) -> Option<String> {
    let entry: SummaryCacheFile = read_json_or_default(path);
    if entry.version != OPTIMIZER_VERSION
        || entry.kind != "chunk_summary"
        || entry.summary.trim().is_empty()
    {
        return None;
    }
    let _ = touch_file(path);
    Some(entry.summary)
}

pub(super) fn save_chunk_summary(
    path: &Path,
    summary: &str,
    config: &ProxyOptimizationConfig,
) -> Result<(), String> {
    let now = now_millis();
    let entry = SummaryCacheFile {
        version: OPTIMIZER_VERSION.to_string(),
        kind: "chunk_summary".to_string(),
        summary: summary.to_string(),
        created_at_ms: now,
        last_used_at_ms: now,
    };
    save_pretty_json(path, &entry)?;
    prune_chunk_cache(config)
}

fn touch_file(path: &Path) -> std::io::Result<()> {
    let file = fs::OpenOptions::new().write(true).open(path)?;
    file.set_modified(SystemTime::now())
}

pub(crate) fn proxy_cache_dir() -> PathBuf {
    claudie_home().join("proxy_cache")
}

pub(super) fn chunk_cache_dir() -> PathBuf {
    proxy_cache_dir().join("chunks")
}

pub(super) fn prune_chunk_cache(config: &ProxyOptimizationConfig) -> Result<(), String> {
    prune_dir_throttled(
        &chunk_cache_dir(),
        config.chunk_cache_ttl_hours,
        config.chunk_cache_max_entries,
        config.cache_max_bytes,
        &LAST_CHUNK_PRUNE_MS,
    )
}

fn prune_dir_throttled(
    dir: &Path,
    ttl_hours: u64,
    max_entries: usize,
    max_bytes: u64,
    last_prune_ms: &AtomicU64,
) -> Result<(), String> {
    let now = now_millis() as u64;
    let last = last_prune_ms.load(Ordering::Relaxed);
    if last > 0 && now.saturating_sub(last) < PRUNE_THROTTLE_MS {
        return Ok(());
    }
    if last_prune_ms
        .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return Ok(());
    }
    prune_cache_dir(dir, ttl_hours, max_entries, max_bytes)
}

pub(crate) fn prune_cache_dir(
    dir: &Path,
    ttl_hours: u64,
    max_entries: usize,
    max_bytes: u64,
) -> Result<(), String> {
    let now = now_millis();
    let ttl_ms = u128::from(ttl_hours).saturating_mul(MILLIS_PER_HOUR);
    let mut entries = cache_dir_entries(dir)?;

    for entry in entries.iter().filter(|entry| {
        ttl_ms > 0
            && entry.last_used_at_ms > 0
            && now.saturating_sub(entry.last_used_at_ms) > ttl_ms
    }) {
        let _ = fs::remove_file(&entry.path);
    }

    entries = cache_dir_entries(dir)?;
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.last_used_at_ms));
    let mut kept_bytes = 0_u64;
    for (index, entry) in entries.iter().enumerate() {
        kept_bytes = kept_bytes.saturating_add(entry.size_bytes);
        let too_many = max_entries > 0 && index >= max_entries;
        let too_large = max_bytes > 0 && kept_bytes > max_bytes;
        if too_many || too_large {
            let _ = fs::remove_file(&entry.path);
        }
    }

    Ok(())
}

#[derive(Clone, Debug)]
struct CacheDirEntry {
    path: PathBuf,
    last_used_at_ms: u128,
    size_bytes: u64,
}

fn cache_dir_entries(dir: &Path) -> Result<Vec<CacheDirEntry>, String> {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for item in read_dir {
        let item = item.map_err(|err| err.to_string())?;
        let path = item.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let metadata = item.metadata().map_err(|err| err.to_string())?;
        if !metadata.is_file() {
            continue;
        }
        let last_used_at_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        entries.push(CacheDirEntry {
            path,
            last_used_at_ms,
            size_bytes: metadata.len(),
        });
    }
    Ok(entries)
}

pub(super) fn chunk_summary_cache_key(
    profile: &LlmProfile,
    request: &Value,
    chunk_messages: &[Value],
    config: &ProxyOptimizationConfig,
) -> String {
    let mut hasher = FnvHasher::new();
    hasher.write_bytes_with_sep(OPTIMIZER_VERSION.as_bytes());
    hasher.write_bytes_with_sep(b"chunk");
    hasher.write_bytes_with_sep(profile.id.as_bytes());
    hasher.write_bytes_with_sep(
        request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .as_bytes(),
    );
    hasher.write_bytes_with_sep(config.signature().as_bytes());
    for message in chunk_messages {
        let _ = serde_json::to_writer(&mut hasher, message);
        hasher.write_separator();
    }
    hasher.write_separator();
    if let Some(tools) = request.get("tools") {
        let _ = serde_json::to_writer(&mut hasher, tools);
    }
    hasher.write_separator();
    hasher.finish_hex()
}

struct FnvHasher(u64);

impl FnvHasher {
    fn new() -> Self {
        Self(0xcbf29ce484222325)
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    fn write_separator(&mut self) {
        self.0 ^= 0xff;
        self.0 = self.0.wrapping_mul(0x100000001b3);
    }

    fn write_bytes_with_sep(&mut self, bytes: &[u8]) {
        self.write_bytes(bytes);
        self.write_separator();
    }

    fn finish_hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

impl std::io::Write for FnvHasher {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.write_bytes(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
