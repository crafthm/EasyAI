//! File-backed application log.
//!
//! Job logs are separate: [`crate::jobs::JobLogger`] writes the user-facing
//! transcript of one lifecycle operation, while this logger records what the
//! application itself is doing. Both end up under `<app_data>/logs`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};

const APP_LOG_FILE: &str = "easyai.log";
const MAX_APP_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Credential-bearing names. A key matches when it is one of these or ends in
/// `_<name>` after normalization, so `X-Amz-Signature` and `access_token`
/// both match while `token_type` and `monkey` do not.
const SENSITIVE_KEYS: &[&str] = &[
    "token",
    "access_token",
    "refresh_token",
    "api_key",
    "apikey",
    "secret",
    "client_secret",
    "password",
    "passwd",
    "signature",
    "sig",
    "credential",
    "credentials",
    "private_key",
];

struct FileLogger {
    file: Mutex<File>,
}

impl Log for FileLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {}: {}\n",
            timestamp(),
            record.level(),
            record.target(),
            redact(&record.args().to_string())
        );
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
        if cfg!(debug_assertions) {
            eprint!("{line}");
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

/// Installs the global logger. `EASYAI_LOG` overrides the level (`debug`,
/// `info`, `warn`, …); the default is `info`.
pub fn init(log_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(log_dir)
        .map_err(|err| format!("Failed to create the log directory: {err}"))?;
    let path = log_dir.join(APP_LOG_FILE);
    rotate_if_large(&path, MAX_APP_LOG_BYTES);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("Failed to open {}: {err}", path.display()))?;
    log::set_boxed_logger(Box::new(FileLogger {
        file: Mutex::new(file),
    }))
    .map_err(|err| format!("Failed to install the logger: {err}"))?;
    let level = std::env::var("EASYAI_LOG")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(LevelFilter::Info);
    log::set_max_level(level);
    Ok(())
}

/// Current UTC time as `YYYY-MM-DD HH:MM:SS`.
pub fn timestamp() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

/// Renames `path` to `<path>.1` when it exceeds `max_bytes`, replacing the
/// previous rotated file.
pub fn rotate_if_large(path: &Path, max_bytes: u64) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.len() <= max_bytes {
        return;
    }
    let extension = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => format!("{ext}.1"),
        None => "1".to_string(),
    };
    let previous = path.with_extension(extension);
    let _ = std::fs::remove_file(&previous);
    let _ = std::fs::rename(path, previous);
}

/// Masks credentials in a log line.
///
/// Handles `key=value` / `key: value` assignments (`token=`, `X-Amz-Signature=`,
/// …), flags whose value is the next word (`--api-key secret`), and
/// `Authorization:` headers, which are masked to the end of the line.
pub fn redact(input: &str) -> String {
    if let Some(start) = authorization_value_start(input) {
        return format!("{}***", &input[..start]);
    }
    let mut out = String::with_capacity(input.len());
    let mut mask_next = false;
    for part in input.split_inclusive(|c: char| c.is_whitespace()) {
        let word_len = part.trim_end().len();
        let (word, trailing) = part.split_at(word_len);
        if mask_next && !word.is_empty() {
            out.push_str("***");
            out.push_str(trailing);
            mask_next = false;
            continue;
        }
        let redacted = redact_assignments(word);
        mask_next = redacted == word && (is_sensitive_flag(word) || is_sensitive_prefix(word));
        out.push_str(&redacted);
        out.push_str(trailing);
    }
    out
}

fn redact_assignments(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut rest = word;
    loop {
        let Some(pos) = rest.find(|c: char| c == '=' || c == ':') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..=pos]);
        let key_start = rest[..pos]
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .map(|index| index + 1)
            .unwrap_or(0);
        if !is_sensitive_key(&rest[key_start..pos]) {
            rest = &rest[pos + 1..];
            if rest.is_empty() {
                break;
            }
            continue;
        }
        let mut value = &rest[pos + 1..];
        let spaces = value.len() - value.trim_start_matches([' ', '\t']).len();
        out.push_str(&value[..spaces]);
        value = &value[spaces..];
        let (quote, inner) = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => (Some(quote), &value[quote.len_utf8()..]),
            _ => (None, value),
        };
        let end = match quote {
            Some(quote) => inner.find(quote).unwrap_or(inner.len()),
            None => inner
                .find(|c: char| matches!(c, ' ' | '\t' | '&' | ',' | ';' | '"' | '\'' | ')' | ']'))
                .unwrap_or(inner.len()),
        };
        if end > 0 {
            out.push_str("***");
        }
        if quote.is_some() {
            out.push_str(&inner[end..]);
            break;
        }
        rest = &inner[end..];
        if rest.is_empty() {
            break;
        }
    }
    out
}

fn is_sensitive_flag(word: &str) -> bool {
    word.starts_with('-') && !word.contains('=') && is_sensitive_key(word)
}

/// Matches a bare assignment prefix such as `password:` or `token=`, whose
/// value is the next whitespace-separated word.
fn is_sensitive_prefix(word: &str) -> bool {
    word.strip_suffix(':')
        .or_else(|| word.strip_suffix('='))
        .is_some_and(is_sensitive_key)
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key
        .trim()
        .trim_start_matches('-')
        .to_ascii_lowercase()
        .replace('-', "_");
    SENSITIVE_KEYS.contains(&key.as_str())
        || key
            .rsplit('_')
            .next()
            .is_some_and(|last| SENSITIVE_KEYS.contains(&last))
}

fn authorization_value_start(input: &str) -> Option<usize> {
    let lower = input.to_ascii_lowercase();
    let start = lower.find("authorization")?;
    let bytes = input.as_bytes();
    let mut index = start + "authorization".len();
    while index < bytes.len() && (bytes[index] == b' ' || bytes[index] == b'\t') {
        index += 1;
    }
    if index >= bytes.len() || (bytes[index] != b':' && bytes[index] != b'=') {
        return None;
    }
    index += 1;
    while index < bytes.len() && (bytes[index] == b' ' || bytes[index] == b'\t') {
        index += 1;
    }
    Some(index)
}

#[cfg(test)]
mod tests {
    use super::{redact, rotate_if_large, timestamp};

    #[test]
    fn timestamp_has_the_expected_shape() {
        let value = timestamp();
        assert_eq!(value.len(), 19, "{value}");
        assert_eq!(value.as_bytes()[4], b'-');
        assert_eq!(value.as_bytes()[10], b' ');
        assert_eq!(value.as_bytes()[13], b':');
    }

    #[test]
    fn redacts_assignments_and_flags() {
        assert_eq!(
            redact("curl https://example.test/a?token=abc&x=1"),
            "curl https://example.test/a?token=***&x=1"
        );
        assert_eq!(
            redact("--api-key supersecret --verbose"),
            "--api-key *** --verbose"
        );
        assert_eq!(
            redact("password: hunter2 secret=abc"),
            "password: *** secret=***"
        );
        assert_eq!(
            redact("X-Amz-Signature=deadbeef&X-Amz-Credential=abc"),
            "X-Amz-Signature=***&X-Amz-Credential=***"
        );
    }

    #[test]
    fn redacts_authorization_headers() {
        assert_eq!(
            redact("Authorization: Bearer abc.def.ghi"),
            "Authorization: ***"
        );
    }

    #[test]
    fn leaves_benign_words_alone() {
        let line = "downloaded monkey=banana token_type=Bearer at 12:00";
        assert_eq!(redact(line), line);
    }

    #[test]
    fn preserves_non_ascii_text() {
        assert_eq!(redact("下载完成 token=abc"), "下载完成 token=***");
    }

    #[test]
    fn rotation_keeps_one_previous_file() {
        let dir = std::env::temp_dir().join(format!(
            "easyai-log-rotate-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("easyai.log");
        std::fs::write(&path, vec![b'x'; 32]).unwrap();
        rotate_if_large(&path, 16);
        assert!(!path.exists());
        assert!(dir.join("easyai.log.1").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}