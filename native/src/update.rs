//! Updates. Nimbus asks its own server for the newest version, checks the signature on what
//! it offers, downloads the installer, checks its hash, and puts a button in the title bar;
//! one click installs it and starts the new version.
//!
//! The server only has to serve files (`<base>/latest.json` and the installer). What it says
//! is trusted only if it is signed with the author's release key: the public half is in this
//! file, the private half stays on the author's PC (see `src/bin/release.rs`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;
use sha2::{Digest, Sha256};
use slint::Weak;

use crate::{on_ui, AppWindow, SplashWindow};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The newest release of the public repository: GitHub serves the files of the latest release at this address.
const BASE: &str = "https://github.com/debugown/Nimbus/releases/latest/download";
/// The public half of the release key.
const PUBLIC_KEY: [u8; 32] = include!("release_public.key");
/// The first check, a little after start-up, then every hour.
const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(3600);
/// Nobody offers an installer bigger than this.
const MAX_BYTES: u64 = 400 * 1024 * 1024;

/// An installer that is downloaded and verified, waiting for the click.
static READY: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);

/// What a release is signed over: the version, the file and its hash, so none can be swapped.
pub fn signed_text(version: &str, file: &str, sha256: &str) -> String {
    format!("nimbus-update\n{version}\n{file}\n{sha256}")
}

/// Is `a` a newer version than `b`? ("0.1.10" is newer than "0.1.9".)
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (a, b) = (parts(a), parts(b));
    (0..a.len().max(b.len())).map(|i| (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0))).find(|(x, y)| x != y).is_some_and(|(x, y)| x > y)
}

fn file_name_ok(name: &str) -> bool {
    name.strip_prefix("Nimbus-Setup-")
        .and_then(|rest| rest.strip_suffix(".exe"))
        .is_some_and(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

/// A release as the server describes it, once its signature is right.
struct Release {
    version: String,
    file: String,
    sha256: String,
}

fn verify(manifest: &Value, key: &[u8; 32]) -> Result<Release, String> {
    let text = |k: &str| manifest[k].as_str().map(str::to_string).ok_or_else(|| format!("в latest.json нет «{k}»"));
    let (version, file, sha256, sig) = (text("version")?, text("file")?, text("sha256")?, text("sig")?);
    if !file_name_ok(&file) || sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("в latest.json странное имя файла или хеш".into());
    }
    let sig = base64::engine::general_purpose::STANDARD.decode(sig.trim()).map_err(|e| e.to_string())?;
    let sig = Signature::from_slice(&sig).map_err(|e| e.to_string())?;
    let key = VerifyingKey::from_bytes(key).map_err(|e| e.to_string())?;
    key.verify(signed_text(&version, &file, &sha256).as_bytes(), &sig).map_err(|_| "подпись обновления неверна".to_string())?;
    Ok(Release { version, file, sha256: sha256.to_lowercase() })
}

fn base() -> String {
    std::env::var("NIMBUS_UPDATE_URL").unwrap_or_else(|_| BASE.to_string()).trim_end_matches('/').to_string()
}

/// The newest release, when it is newer than this copy.
async fn newest(http: &reqwest::Client) -> Result<Option<Release>, String> {
    let base = base();
    let res = http.get(format!("{base}/latest.json")).timeout(Duration::from_secs(20)).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("сервер обновлений ответил {}", res.status()));
    }
    let manifest: Value = res.json().await.map_err(|e| e.to_string())?;
    let release = verify(&manifest, &PUBLIC_KEY)?;
    Ok(newer(&release.version, VERSION).then_some(release))
}

/// Looks for a newer release; downloads it into the temp folder when there is one.
async fn check(http: &reqwest::Client) -> Result<Option<(String, PathBuf)>, String> {
    let Some(release) = newest(http).await? else { return Ok(None) };
    let path = fetch(http, &release, |_, _| {}).await?;
    Ok(Some((release.version, path)))
}

/// The installer of a release, downloaded and its hash checked. `progress` hears the bytes so far and the total, if known.
async fn fetch(http: &reqwest::Client, release: &Release, mut progress: impl FnMut(u64, Option<u64>)) -> Result<PathBuf, String> {
    let base = base();
    let dir = std::env::temp_dir().join("Nimbus-Update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&release.file);
    // Already downloaded and still right: no need again.
    if std::fs::read(&path).map(|b| hex(&Sha256::digest(b))).is_ok_and(|h| h == release.sha256) {
        return Ok(path);
    }
    let mut res = http.get(format!("{base}/{}", release.file)).timeout(Duration::from_secs(900)).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("установщик не скачался: {}", res.status()));
    }
    let total = res.content_length();
    let part = path.with_extension("part");
    let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let (mut hash, mut size) = (Sha256::new(), 0u64);
    while let Some(chunk) = res.chunk().await.map_err(|e| e.to_string())? {
        size += chunk.len() as u64;
        if size > MAX_BYTES {
            let _ = std::fs::remove_file(&part);
            return Err("установщик слишком большой".into());
        }
        hash.update(&chunk);
        std::io::Write::write_all(&mut out, &chunk).map_err(|e| e.to_string())?;
        progress(size, total);
    }
    drop(out);
    if hex(&hash.finalize()) != release.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err("установщик не совпал с подписанным хешем, он не будет запущен".into());
    }
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// How long the window waits for the answer about updates before opening the app anyway.
const START_CHECK: Duration = Duration::from_secs(5);
/// The window stays at least this long, so it doesn't just blink.
const START_SHOWN: Duration = Duration::from_millis(1100);
const NO_CONNECTION_SHOWN: Duration = Duration::from_millis(1200);

/// The phases of the start window (`SplashWindow.phase`).
const CHECKING: i32 = 0;
const DOWNLOADING: i32 = 1;
const INSTALLING: i32 = 2;
const STARTING: i32 = 3;
const NO_CONNECTION: i32 = 4;

fn on_splash(splash: &Weak<SplashWindow>, phase: i32, progress: f32, detail: String) {
    let _ = splash.upgrade_in_event_loop(move |s| {
        s.set_phase(phase);
        s.set_progress(progress);
        s.set_detail(detail.into());
    });
}

/// "Версия 0.1.6, 12 из 34 МБ".
fn download_detail(version: &str, got: u64, total: Option<u64>) -> String {
    let mb = |n: u64| n / (1024 * 1024);
    match total {
        Some(total) => crate::i18n::tf("Версия {}, {} из {} МБ", &[&version, &mb(got), &mb(total)]),
        None => crate::i18n::tf("Версия {}, {} МБ", &[&version, &mb(got)]),
    }
}

/// Before the app opens: installs a newer release if there is one (this copy then exits, and the installer starts the new
/// one). Returns when the app should open: nothing newer, no answer in time, a failed download, or the person skipped it.
pub async fn at_start(splash: Weak<SplashWindow>, skip: Arc<AtomicBool>) {
    let http = reqwest::Client::new();
    let started = Instant::now();
    on_splash(&splash, CHECKING, 0.0, crate::i18n::tf("Версия {}", &[&VERSION]));
    let asked = tokio::time::timeout(START_CHECK, newest(&http)).await;
    tokio::time::sleep(START_SHOWN.saturating_sub(started.elapsed())).await;
    let release = match asked {
        Ok(Ok(Some(release))) => release,
        Ok(Ok(None)) => {
            on_splash(&splash, STARTING, 0.0, String::new());
            return;
        }
        Ok(Err(e)) => {
            crate::log_line(&format!("update at start: {e}"));
            on_splash(&splash, NO_CONNECTION, 0.0, crate::i18n::t("Запускаю текущую версию"));
            tokio::time::sleep(NO_CONNECTION_SHOWN).await;
            return;
        }
        Err(_) => {
            crate::log_line("update at start: no answer in time");
            on_splash(&splash, NO_CONNECTION, 0.0, crate::i18n::t("Запускаю текущую версию"));
            tokio::time::sleep(NO_CONNECTION_SHOWN).await;
            return;
        }
    };
    on_splash(&splash, DOWNLOADING, 0.0, download_detail(&release.version, 0, None));
    let mut shown_percent = 0;
    let download = fetch(&http, &release, |got, total| {
        let percent = total.map_or(0, |t| got * 100 / t.max(1));
        if percent != shown_percent {
            shown_percent = percent;
            on_splash(&splash, DOWNLOADING, percent as f32 / 100.0, download_detail(&release.version, got, total));
        }
    });
    let skipped = async {
        while !skip.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    let path = tokio::select! {
        done = download => done,
        _ = skipped => Err("skipped".to_string()),
    };
    match path {
        Ok(path) => {
            on_splash(&splash, INSTALLING, 1.0, crate::i18n::t("Nimbus перезапустится сам"));
            run_installer(&release.version, &path);
        }
        Err(e) => crate::log_line(&format!("update at start: {e}")),
    }
    on_splash(&splash, STARTING, 0.0, String::new());
}

/// The installer runs quietly, closes this copy, installs and starts the new one.
fn run_installer(version: &str, path: &std::path::Path) {
    crate::log_line(&format!("update: installing {version}"));
    match std::process::Command::new(path).arg("/S").spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => crate::log_line(&format!("update: could not start the installer: {e}")),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// In the background: look for an update soon after start, and every hour.
pub async fn watch(ui: Weak<AppWindow>) {
    let http = reqwest::Client::new();
    tokio::time::sleep(FIRST_CHECK).await;
    loop {
        match check(&http).await {
            Ok(Some((version, path))) => {
                crate::log_line(&format!("update: {version} is ready"));
                let text = crate::i18n::tf("Вышла версия {}. Обновить сейчас?", &[&version]);
                *READY.lock().unwrap() = Some((version, path));
                on_ui(&ui, move |ui| ui.set_update_text(text.into()));
            }
            Ok(None) => {}
            Err(e) => crate::log_line(&format!("update: {e}")),
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

/// The click on the button: the installer runs quietly, closes this copy, installs and starts the new one.
/// The button stays if that fails, so the release is kept for another click.
pub fn install() {
    let Some((version, path)) = READY.lock().unwrap().clone() else { return };
    run_installer(&version, &path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn download_is_told_in_megabytes() {
        assert!(download_detail("0.1.6", 12 * 1024 * 1024, Some(34 * 1024 * 1024)).contains("12"));
        assert!(download_detail("0.1.6", 12 * 1024 * 1024, Some(34 * 1024 * 1024)).contains("34"));
        assert!(download_detail("0.1.6", 5 * 1024 * 1024, None).contains('5'));
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.1.10", "0.1.9"));
        assert!(newer("0.2", "0.1.9"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(!newer("0.1.1", "0.1.1"));
        assert!(!newer("0.1.0", "0.1.1"));
        assert!(!newer("0.1", "0.1.0"));
    }

    #[test]
    fn only_our_installers_are_named() {
        assert!(file_name_ok("Nimbus-Setup-0.1.2.exe"));
        assert!(!file_name_ok("Nimbus-Setup-0.1.2.exe.bat"));
        assert!(!file_name_ok("..\\evil.exe"));
        assert!(!file_name_ok("Nimbus-Setup-.exe"));
        assert!(!file_name_ok("other.exe"));
    }

    fn manifest(key: &SigningKey, version: &str, file: &str, sha: &str) -> Value {
        let sig = key.sign(signed_text(version, file, sha).as_bytes());
        serde_json::json!({ "version": version, "file": file, "sha256": sha, "sig": base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()) })
    }

    #[test]
    fn a_signed_release_passes_and_a_changed_one_does_not() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let public = key.verifying_key().to_bytes();
        let sha = "a".repeat(64);
        let good = manifest(&key, "0.1.2", "Nimbus-Setup-0.1.2.exe", &sha);
        let release = verify(&good, &public).unwrap();
        assert_eq!((release.version.as_str(), release.file.as_str()), ("0.1.2", "Nimbus-Setup-0.1.2.exe"));

        // The same signature on another hash, version or file is refused.
        for (field, value) in [("sha256", "b".repeat(64)), ("version", "9.9.9".to_string()), ("file", "Nimbus-Setup-9.9.9.exe".to_string())] {
            let mut tampered = good.clone();
            tampered[field] = Value::String(value);
            assert!(verify(&tampered, &public).is_err(), "{field} changed");
        }
        // A key that isn't ours is refused too.
        let other = SigningKey::from_bytes(&[8u8; 32]).verifying_key().to_bytes();
        assert!(verify(&good, &other).is_err());
    }
}
