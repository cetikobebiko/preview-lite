//! Uygulama ayarları (`app_config.json`): son açılan dosyalar ve belge başına görünüm durumu.
//!
//! Dosya Tauri'nin AppData dizinindedir (webview'in eski `BaseDirectory.AppData` yolu ile aynı).
//! İçerik frontend'in JSON nesnesidir; Rust yalnız boyutunu ve bir JSON nesnesi olduğunu
//! doğrular, `document_io::write_atomically` ile yazar. Okunamayan bir dosya silinmez ya da
//! üzerine yazılmaz: kenara alınır ve frontend varsayılanlarla başlar.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::Manager;

const CONFIG_FILE: &str = "app_config.json";
/// Son 20 dosya ve sınırlı sayıda belge durumu bunun çok altında kalır.
pub(crate) const MAX_CONFIG_BYTES: usize = 1024 * 1024;

fn config_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|error| format!("AppData dizini çözümlenemedi: {error}"))
}

/// Kayıtlı ayarları döndürür; dosya yoksa ya da kenara alındıysa `None`.
#[tauri::command]
pub async fn read_app_config(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let dir = config_dir(&app)?;
    // Dosya I/O'su UI thread'ini ve async worker'ları bloklamasın.
    tauri::async_runtime::spawn_blocking(move || read_config_in(&dir))
        .await
        .map_err(|error| format!("Ayar okuma görevi tamamlanamadı: {error}"))?
}

/// Ayarları atomik olarak yazar: dosya ya eski ya da yeni içeriğin tamamını taşır.
#[tauri::command]
pub async fn write_app_config(app: tauri::AppHandle, contents: String) -> Result<(), String> {
    let dir = config_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || write_config_in(&dir, &contents))
        .await
        .map_err(|error| format!("Ayar yazma görevi tamamlanamadı: {error}"))?
}

pub(crate) fn read_config_in(dir: &Path) -> Result<Option<String>, String> {
    let path = dir.join(CONFIG_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Ayar dosyası okunamadı: {error}")),
    };
    match validate_config(&bytes) {
        Ok(()) => Ok(Some(String::from_utf8(bytes).unwrap_or_default())),
        Err(reason) => {
            let quarantine = dir.join(format!("app_config.corrupt-{}.json", unix_millis()));
            fs::rename(&path, &quarantine).map_err(|error| {
                format!("Okunamayan ayar dosyası kenara alınamadı ({reason}): {error}")
            })?;
            tracing::warn!(
                code = "APP_CONFIG_QUARANTINED",
                reason = %reason,
                quarantine = %quarantine.display(),
                "Ayar dosyası okunamadı; kenara alındı, varsayılanlarla devam ediliyor"
            );
            Ok(None)
        }
    }
}

pub(crate) fn write_config_in(dir: &Path, contents: &str) -> Result<(), String> {
    validate_config(contents.as_bytes())?;
    fs::create_dir_all(dir).map_err(|error| format!("AppData dizini oluşturulamadı: {error}"))?;
    crate::document_io::write_atomically(&dir.join(CONFIG_FILE), |staged| {
        std::io::Write::write_all(staged, contents.as_bytes())
    })
    .map_err(|error| format!("Ayar dosyası yazılamadı: {error}"))
}

fn validate_config(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(format!(
            "ayar içeriği {} bayt; sınır {MAX_CONFIG_BYTES} bayt",
            bytes.len()
        ));
    }
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(serde_json::Value::Object(_)) => Ok(()),
        Ok(_) => Err("ayar içeriği bir JSON nesnesi değil".to_string()),
        Err(error) => Err(format!("ayar içeriği geçerli JSON değil: {error}")),
    }
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(dir: &Path) -> Vec<String> {
        let mut names = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn missing_config_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_config_in(dir.path()).unwrap(), None);
    }

    #[test]
    fn write_creates_the_directory_and_round_trips_exact_contents() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("com.preview.lite");
        let contents = "{\n  \"recentFiles\": [\"/a.pdf\"],\n  \"documentStates\": {}\n}";

        write_config_in(&dir, contents).unwrap();

        assert_eq!(read_config_in(&dir).unwrap().as_deref(), Some(contents));
        assert_eq!(entries(&dir), [CONFIG_FILE]);
    }

    #[test]
    fn write_replaces_the_previous_config_whole() {
        let dir = tempfile::tempdir().unwrap();
        write_config_in(dir.path(), &format!("{{\"pad\":\"{}\"}}", "x".repeat(4096))).unwrap();
        write_config_in(dir.path(), "{\"recentFiles\":[]}").unwrap();

        assert_eq!(
            fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap(),
            "{\"recentFiles\":[]}"
        );
        assert_eq!(entries(dir.path()), [CONFIG_FILE]);
    }

    #[test]
    fn write_rejects_non_object_invalid_and_oversized_contents_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        write_config_in(dir.path(), "{\"keep\":true}").unwrap();
        let oversized = format!("{{\"pad\":\"{}\"}}", "x".repeat(MAX_CONFIG_BYTES));

        for contents in ["[]", "\"text\"", "{\"truncated\":", oversized.as_str()] {
            assert!(
                write_config_in(dir.path(), contents).is_err(),
                "{contents:.20}"
            );
        }
        assert_eq!(
            fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap(),
            "{\"keep\":true}"
        );
        assert_eq!(entries(dir.path()), [CONFIG_FILE]);
    }

    #[test]
    fn unreadable_config_is_quarantined_instead_of_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(CONFIG_FILE),
            b"{\"recentFiles\": [\"/a.pdf\"",
        )
        .unwrap();

        assert_eq!(read_config_in(dir.path()).unwrap(), None);

        let names = entries(dir.path());
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(names[0].starts_with("app_config.corrupt-"), "{names:?}");
        assert_eq!(
            fs::read(dir.path().join(&names[0])).unwrap(),
            b"{\"recentFiles\": [\"/a.pdf\""
        );
        // The next save starts a fresh config next to the preserved copy.
        write_config_in(dir.path(), "{}").unwrap();
        assert_eq!(read_config_in(dir.path()).unwrap().as_deref(), Some("{}"));
    }
}
