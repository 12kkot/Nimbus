//! Nimbus's own streaming server (LiveKit and the token service): started with the app,
//! looked after while it runs, stopped when it quits. If the ports already answer (the
//! server was started by hand) nothing is started.

use std::net::{SocketAddr, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::Duration;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// The token service's port: if it answers, the server is up.
const TOKEN_PORT: u16 = 7890;

/// The keys LiveKit and the token service share. Made at random the first time the server is set up,
/// kept in `server\config\keys.json` (not in git), readable only by this Windows user.
#[derive(Debug, Clone, PartialEq)]
pub struct Keys {
    pub key: String,
    pub secret: String,
}

/// `bytes` random bytes from Windows' own generator, as hex.
fn random_hex(bytes: usize) -> Result<String, String> {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut buf = vec![0u8; bytes];
    let status = unsafe { BCryptGenRandom(None, &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status.0 != 0 {
        return Err(format!("Windows не дал случайные числа ({:#x})", status.0));
    }
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn generate() -> Result<Keys, String> {
    Ok(Keys {
        key: format!("nb{}", random_hex(10)?),
        secret: random_hex(32)?,
    })
}

/// Only the current Windows user may read the file (removes the inherited access of other users).
pub fn restrict_to_user(path: &Path) -> bool {
    let Ok(user) = std::env::var("USERNAME") else { return false };
    Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r", &format!("{user}:(R,W)")])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The keys in `dir\config\keys.json`; made (and the file locked down) when there are none yet.
pub fn load_or_create_keys(dir: &Path) -> Result<Keys, String> {
    let file = dir.join("config").join("keys.json");
    if let Ok(bytes) = std::fs::read(&file) {
        let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("keys.json повреждён: {e}"))?;
        let (key, secret) = (v["key"].as_str().unwrap_or(""), v["secret"].as_str().unwrap_or(""));
        if key.is_empty() || secret.len() < 32 {
            return Err("в keys.json пустой ключ или слишком короткий секрет: удали файл, ключи создадутся заново".into());
        }
        return Ok(Keys {
            key: key.into(),
            secret: secret.into(),
        });
    }
    let keys = generate()?;
    std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&file, serde_json::json!({ "key": keys.key, "secret": keys.secret }).to_string()).map_err(|e| e.to_string())?;
    restrict_to_user(&file);
    Ok(keys)
}

/// The programs we started (not the ones started by hand).
static STARTED: Mutex<Vec<Child>> = Mutex::new(Vec::new());

fn answers(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Where the two programs are: `server\` next to the exe, or the folder saved in the settings.
pub fn find_dir(exe_dir: &Path, saved: Option<&str>) -> Option<PathBuf> {
    let has_both = |d: &Path| d.join("livekit").join("livekit-server.exe").is_file() && d.join("nimbus-token.exe").is_file();
    [Some(exe_dir.join("server")), saved.map(PathBuf::from)]
        .into_iter()
        .flatten()
        .find(|d| has_both(d))
}

fn spawn(program: PathBuf, args: &[&str], cwd: &Path, envs: &[(&str, String)]) -> Option<Child> {
    Command::new(program)
        .args(args)
        .envs(envs.iter().map(|(k, v)| (*k, v.as_str())))
        .current_dir(cwd)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .ok()
}

/// Starts what isn't running yet. Returns whether the server answers afterwards (or was already up).
pub fn ensure_running(dir: &Path) -> bool {
    if answers(TOKEN_PORT) {
        return true;
    }
    // Keys made at random for this server (no devkey / secret), given to both programs through
    // their environment, so they aren't on any command line.
    let keys = match load_or_create_keys(dir) {
        Ok(k) => k,
        Err(e) => {
            crate::log_line(&format!("voice server: no keys: {e}"));
            return false;
        }
    };
    let mut started = STARTED.lock().unwrap();
    // What we started earlier and has died is dropped before starting again.
    started.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
    if !answers(7880) {
        let env = [("LIVEKIT_KEYS", format!("{}: {}", keys.key, keys.secret))];
        if let Some(c) = spawn(dir.join("livekit").join("livekit-server.exe"), &["--bind", "0.0.0.0"], dir, &env) {
            started.push(c);
        }
    }
    let env = [("LIVEKIT_API_KEY", keys.key.clone()), ("LIVEKIT_API_SECRET", keys.secret.clone())];
    if let Some(c) = spawn(dir.join("nimbus-token.exe"), &[], dir, &env) {
        started.push(c);
    }
    drop(started);
    std::thread::sleep(Duration::from_millis(1500));
    answers(TOKEN_PORT)
}

/// Stops the programs this copy of Nimbus started.
pub fn stop() {
    for mut c in STARTED.lock().unwrap().drain(..) {
        let _ = c.kill();
    }
}

/// In the background: start the server now, and every half minute make sure it's still there.
pub fn keep_running(dir: PathBuf) {
    std::thread::Builder::new()
        .name("server-keeper".into())
        .spawn(move || loop {
            let up = ensure_running(&dir);
            crate::log_line(if up {
                "voice server: up"
            } else {
                "voice server: could not be started"
            });
            std::thread::sleep(Duration::from_secs(30));
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_counts_only_with_both_programs() {
        let root = std::env::temp_dir().join(format!("nimbus-servers-test-{}", std::process::id()));
        let server = root.join("server");
        std::fs::create_dir_all(server.join("livekit")).unwrap();
        assert_eq!(find_dir(&root, None), None, "nothing in it yet");
        std::fs::write(server.join("livekit").join("livekit-server.exe"), b"x").unwrap();
        assert_eq!(find_dir(&root, None), None, "the token service is missing");
        std::fs::write(server.join("nimbus-token.exe"), b"x").unwrap();
        assert_eq!(find_dir(&root, None), Some(server.clone()));
        // The saved folder is the second choice.
        let elsewhere = std::env::temp_dir().join("nimbus-servers-test-nowhere");
        assert_eq!(find_dir(&elsewhere, Some(server.to_str().unwrap())), Some(server));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn keys_are_long_random_and_different_each_time() {
        let (a, b) = (generate().unwrap(), generate().unwrap());
        assert_eq!(a.key.len(), 22);
        assert_eq!(a.secret.len(), 64);
        assert!(a.secret.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert_ne!(a.secret, b.secret);
        assert!(
            a.secret.chars().collect::<std::collections::HashSet<_>>().len() > 8,
            "not a repeated digit"
        );
    }

    #[test]
    fn keys_are_made_once_kept_and_locked_to_the_user() {
        let dir = std::env::temp_dir().join(format!("nimbus-keys-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = load_or_create_keys(&dir).unwrap();
        let again = load_or_create_keys(&dir).unwrap();
        assert_eq!(first, again, "a second start finds the same keys");
        let file = dir.join("config").join("keys.json");
        let out = Command::new("icacls").arg(&file).creation_flags(CREATE_NO_WINDOW).output().unwrap();
        let acl = String::from_utf8_lossy(&out.stdout).to_lowercase();
        let user = std::env::var("USERNAME").unwrap().to_lowercase();
        assert!(acl.contains(&user), "the user can read it: {acl}");
        assert!(
            !acl.contains("everyone") && !acl.contains("builtin\\users"),
            "no one else can: {acl}"
        );
        // A short or empty secret is refused, not used.
        std::fs::write(&file, r#"{"key":"k","secret":"short"}"#).unwrap();
        assert!(load_or_create_keys(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_closed_port_does_not_answer() {
        assert!(!answers(1));
    }
}
