use rhai::{Dynamic, Engine, Scope, AST};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::Mutex;
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub permissions: Vec<String>, // e.g. "fs:read", "ai:summarize"
    pub main_script: String,
}

pub struct ActivePlugin {
    pub manifest: PluginManifest,
    pub ast: AST,
}

pub struct PluginState {
    pub engine: Engine,
    pub plugins: Mutex<HashMap<String, ActivePlugin>>,
}

impl PluginState {
    pub fn new() -> Self {
        let mut engine = Engine::new();

        // Güvenlik ve Sandbox (Kum Havuzu) Ayarları
        // Sonsuz döngü veya aşırı CPU kullanımını engellemek için işlem sınırları:
        engine.set_max_operations(500_000);
        engine.set_max_expr_depths(50, 50);

        // İleri aşamalarda engine.register_fn ile özel fonksiyonlar Rust'tan Rhai'ye aktarılabilir.

        PluginState {
            engine,
            plugins: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for PluginState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[tauri::command]
pub fn load_plugin(
    manifest_path: String,
    state: State<'_, PluginState>,
) -> Result<PluginInfo, String> {
    // 1. Manifest Oku
    let manifest_content =
        fs::read_to_string(&manifest_path).map_err(|e| format!("Manifest okuma hatası: {}", e))?;
    let manifest: PluginManifest = serde_json::from_str(&manifest_content)
        .map_err(|e| format!("JSON ayrıştırma hatası: {}", e))?;

    // 2. Script Oku
    let mut script_path = std::path::PathBuf::from(&manifest_path);
    script_path.pop(); // bulunduğu klasöre in
    script_path.push(&manifest.main_script);

    let script_content =
        fs::read_to_string(&script_path).map_err(|e| format!("Script okuma hatası: {}", e))?;

    // 3. Rhai AST Derle
    let ast = state
        .engine
        .compile(&script_content)
        .map_err(|e| format!("Derleme (Syntax) hatası: {}", e))?;

    let info = PluginInfo {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
    };

    // 4. Sisteme Ekle
    let mut plugins_lock = state
        .plugins
        .lock()
        .map_err(|_| "Sistem durumu kilitlendi (Mutex Poison Error)")?;
    plugins_lock.insert(manifest.id.clone(), ActivePlugin { manifest, ast });

    Ok(info)
}

#[tauri::command]
pub fn unload_plugin(plugin_id: String, state: State<'_, PluginState>) -> Result<(), String> {
    let mut plugins_lock = state
        .plugins
        .lock()
        .map_err(|_| "Sistem durumu kilitlendi (Mutex Poison Error)")?;
    if plugins_lock.remove(&plugin_id).is_some() {
        Ok(())
    } else {
        Err("Eklenti bulunamadı".into())
    }
}

#[tauri::command]
pub fn get_active_plugins(state: State<'_, PluginState>) -> Result<Vec<PluginInfo>, String> {
    let plugins_lock = state
        .plugins
        .lock()
        .map_err(|_| "Sistem durumu kilitlendi (Mutex Poison Error)")?;
    let list = plugins_lock
        .values()
        .map(|p| PluginInfo {
            id: p.manifest.id.clone(),
            name: p.manifest.name.clone(),
            version: p.manifest.version.clone(),
        })
        .collect();
    Ok(list)
}

#[tauri::command]
pub fn trigger_plugin_hook(
    plugin_id: String,
    hook_name: String,
    payload: String,
    state: State<'_, PluginState>,
) -> Result<String, String> {
    let plugins_lock = state
        .plugins
        .lock()
        .map_err(|_| "Sistem durumu kilitlendi (Mutex Poison Error)")?;

    let plugin = plugins_lock.get(&plugin_id).ok_or("Eklenti bulunamadı")?;

    // Yalnızca okuma izin kontrolü vb yapılabilir
    // if !plugin.manifest.permissions.contains(&"hook:run".to_string()) { ... }

    let mut scope = Scope::new();
    scope.push("event_payload", payload);

    // Eklenti scriptinde tanımlı olan fonksiyonu çağır
    match state
        .engine
        .call_fn::<String>(&mut scope, &plugin.ast, &hook_name, ())
    {
        Ok(res) => Ok(res),
        Err(e) => {
            // Fonksiyon bulunamadıysa ignore edilebilir veya hata dönülebilir
            // Biz basitçe hata dönüyoruz
            Err(format!("Eklenti hatası: {}", e))
        }
    }
}

#[tauri::command]
pub fn run_script_direct(
    script_content: String,
    state: State<'_, PluginState>,
) -> Result<String, String> {
    // Konsol (Developer Console) için anlık kod çalıştırma
    let mut scope = Scope::new();
    match state
        .engine
        .eval_with_scope::<Dynamic>(&mut scope, &script_content)
    {
        Ok(result) => Ok(result.to_string()),
        Err(e) => Err(format!("Script hatası: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rhai_sandbox() {
        let state = PluginState::new();
        // Sonsuz döngü yazalım, max_operations limitine çarpması lazım.
        let bad_script = "while true { let a = 1; }";
        let mut scope = Scope::new();
        let res = state.engine.eval_with_scope::<()>(&mut scope, bad_script);
        assert!(res.is_err());
        let err_str = res.unwrap_err().to_string();
        assert!(err_str.contains("operations"));
    }
}
