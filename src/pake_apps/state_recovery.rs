use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct AppRunningState {
    pub app_ids: Vec<String>,
    pub timestamp: u64,
}

impl AppRunningState {
    pub fn new(app_ids: Vec<String>) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Self { app_ids, timestamp }
    }

    fn state_file() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/root/.config"))
            .join("ivnc")
            .join("app_running_state.json")
    }

    /// Save running apps state to file
    pub fn save(&self) -> Result<(), String> {
        let path = Self::state_file();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize state: {}", e))?;

        std::fs::write(&path, json)
            .map_err(|e| format!("Failed to write state file: {}", e))?;

        log::info!("Saved running apps state: {:?}", self.app_ids);
        Ok(())
    }

    /// Load running apps state from file
    pub fn load() -> Result<Self, String> {
        let path = Self::state_file();
        if !path.exists() {
            return Err("State file does not exist".to_string());
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read state file: {}", e))?;

        let state: Self = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse state file: {}", e))?;

        log::info!("Loaded running apps state: {:?}", state.app_ids);
        Ok(state)
    }

    /// Clear the state file
    pub fn clear() -> Result<(), String> {
        let path = Self::state_file();
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("Failed to remove state file: {}", e))?;
            log::info!("Cleared running apps state file");
        }
        Ok(())
    }

    /// Check if state is recent (within last 5 minutes)
    pub fn is_recent(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Consider state recent if it's within 5 minutes
        now - self.timestamp < 300
    }
}
