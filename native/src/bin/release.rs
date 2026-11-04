//! The author's release tool (not part of the app): makes the release key and signs a release.
//!
//!   release keygen                         makes the key, writes its public half to src\release_public.key
//!   release publish <installer> <version>  copies the installer and writes latest.json, signed, into dist\update
//!
//! The private key lives in %APPDATA%\Nimbus Native\release.key, outside the project.

use std::path::PathBuf;

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};

fn key_file() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").expect("APPDATA")).join("Nimbus Native").join("release.key")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn load_key() -> Result<SigningKey, String> {
    let text = std::fs::read_to_string(key_file()).map_err(|e| format!("нет ключа ({e}); сначала: release keygen"))?;
    let bytes: Vec<u8> = (0..text.trim().len() / 2).map(|i| u8::from_str_radix(&text.trim()[2 * i..2 * i + 2], 16).unwrap_or(0)).collect();
    let seed: [u8; 32] = bytes.try_into().map_err(|_| "ключ повреждён".to_string())?;
    Ok(SigningKey::from_bytes(&seed))
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            if key_file().exists() {
                return Err(format!("ключ уже есть: {} (удали его сам, если нужен новый; тогда старые установки перестанут обновляться)", key_file().display()));
            }
            let mut seed = [0u8; 32];
            let status = unsafe { BCryptGenRandom(None, &mut seed, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
            if status.0 != 0 {
                return Err("Windows не дал случайные числа".into());
            }
            std::fs::create_dir_all(key_file().parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(key_file(), hex(&seed)).map_err(|e| e.to_string())?;
            let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
            let list = public.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ");
            let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("release_public.key");
            std::fs::write(&target, format!("[{list}]\n")).map_err(|e| e.to_string())?;
            println!("Ключ создан: {}", key_file().display());
            println!("Открытая часть записана в {}: пересобери приложение.", target.display());
            Ok(())
        }
        Some("publish") => {
            let (installer, version) = (args.get(1).ok_or("release publish <установщик> <версия>")?, args.get(2).ok_or("release publish <установщик> <версия>")?);
            let notes = args.get(3).cloned().unwrap_or_default();
            let bytes = std::fs::read(installer).map_err(|e| e.to_string())?;
            let sha = hex(&Sha256::digest(&bytes));
            let file = format!("Nimbus-Setup-{version}.exe");
            let signed = format!("nimbus-update\n{version}\n{file}\n{sha}");
            let sig = base64::engine::general_purpose::STANDARD.encode(load_key()?.sign(signed.as_bytes()).to_bytes());
            let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("dist").join("update");
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            std::fs::write(out.join(&file), &bytes).map_err(|e| e.to_string())?;
            let manifest = serde_json::json!({ "version": version, "file": file, "sha256": sha, "sig": sig, "notes": notes });
            std::fs::write(out.join("latest.json"), serde_json::to_string_pretty(&manifest).unwrap()).map_err(|e| e.to_string())?;
            println!("Готово: {}", out.display());
            println!("Залей на сервер: latest.json и {file}");
            Ok(())
        }
        _ => Err("release keygen | release publish <установщик> <версия> [заметки]".into()),
    }
}
