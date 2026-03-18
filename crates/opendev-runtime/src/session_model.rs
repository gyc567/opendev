//! Per-session model configuration overlay.
//!
//! Stores a sparse map in session metadata with only the slots the user
//! explicitly set. Missing keys fall through to global config.
//!
//! Precedence: session-model > project config > global config > defaults
//!
//! Ported from `opendev/core/runtime/session_model.py`.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// The set of field names that are valid session-model overlay keys.
pub static SESSION_MODEL_FIELDS: &[&str] = &[
    "model",
    "model_provider",
    "model_thinking",
    "model_thinking_provider",
    "model_vlm",
    "model_vlm_provider",
];

/// A session-model overlay: sparse key-value map of config overrides.
pub type SessionOverlay = HashMap<String, String>;

/// Manages the session-model overlay lifecycle.
///
/// Tracks original config values so we can:
/// - Restore before save_config() to prevent leaking overlay to settings.json
/// - Revert on /session-model clear or /clear
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionModelManager {
    /// Original values that were overridden.
    originals: HashMap<String, String>,
    /// Currently active overlay (None = no overlay).
    active_overlay: Option<SessionOverlay>,
}

impl SessionModelManager {
    /// Create a new manager with no active overlay.
    pub fn new() -> Self {
        Self {
            originals: HashMap::new(),
            active_overlay: None,
        }
    }

    /// Whether a session-model overlay is currently active.
    pub fn is_active(&self) -> bool {
        self.active_overlay.as_ref().is_some_and(|o| !o.is_empty())
    }

    /// Apply an overlay, recording the original values from the provided config getter.
    ///
    /// The `get_config_value` closure retrieves the current config value for a given key.
    /// The `set_config_value` closure applies the override.
    pub fn apply<G, S>(
        &mut self,
        overlay: &SessionOverlay,
        get_config_value: G,
        set_config_value: S,
    ) where
        G: Fn(&str) -> Option<String>,
        S: Fn(&str, &str),
    {
        if overlay.is_empty() {
            return;
        }

        let valid_fields: HashSet<&str> = SESSION_MODEL_FIELDS.iter().copied().collect();

        self.active_overlay = Some(overlay.clone());
        self.originals.clear();

        for (key, value) in overlay {
            if !valid_fields.contains(key.as_str()) {
                continue;
            }
            if let Some(original) = get_config_value(key) {
                self.originals.insert(key.clone(), original);
            }
            set_config_value(key, value);
        }
    }

    /// Restore original config values, removing the overlay.
    ///
    /// The `set_config_value` closure applies each restored value.
    pub fn restore<S>(&mut self, set_config_value: S)
    where
        S: Fn(&str, &str),
    {
        for (key, value) in &self.originals {
            set_config_value(key, value);
        }
        self.originals.clear();
        self.active_overlay = None;
    }

    /// Return the current overlay dict (for persistence).
    pub fn get_overlay(&self) -> Option<&SessionOverlay> {
        self.active_overlay.as_ref()
    }
}

impl Default for SessionModelManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Read session-model overlay from session metadata.
pub fn get_session_model(metadata: &serde_json::Value) -> Option<SessionOverlay> {
    metadata
        .get("session_model")
        .and_then(|v| serde_json::from_value::<SessionOverlay>(v.clone()).ok())
}

/// Write session-model overlay to session metadata.
pub fn set_session_model(metadata: &mut serde_json::Value, overlay: &SessionOverlay) {
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert(
            "session_model".to_string(),
            serde_json::to_value(overlay).unwrap_or_default(),
        );
    }
}

/// Remove session-model overlay from session metadata.
pub fn clear_session_model(metadata: &mut serde_json::Value) {
    if let Some(obj) = metadata.as_object_mut() {
        obj.remove("session_model");
    }
}

/// Validate overlay entries against valid field names.
///
/// Returns `(valid_overlay, warnings)`.
pub fn validate_session_model(overlay: &SessionOverlay) -> (SessionOverlay, Vec<String>) {
    if overlay.is_empty() {
        return (HashMap::new(), Vec::new());
    }

    let valid_fields: HashSet<&str> = SESSION_MODEL_FIELDS.iter().copied().collect();
    let mut valid = HashMap::new();
    let mut warnings = Vec::new();

    for (key, value) in overlay {
        if valid_fields.contains(key.as_str()) {
            valid.insert(key.clone(), value.clone());
        } else {
            warnings.push(format!("Unknown session-model field '{}', ignored", key));
        }
    }

    (valid, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn test_new_manager_inactive() {
        let mgr = SessionModelManager::new();
        assert!(!mgr.is_active());
        assert!(mgr.get_overlay().is_none());
    }

    #[test]
    fn test_apply_overlay() {
        let mut mgr = SessionModelManager::new();
        let config: RefCell<HashMap<String, String>> = RefCell::new(HashMap::from([
            ("model".into(), "gpt-4".into()),
            ("model_provider".into(), "openai".into()),
        ]));

        let overlay: SessionOverlay = HashMap::from([
            ("model".into(), "claude-3-opus".into()),
            ("model_provider".into(), "anthropic".into()),
        ]);

        mgr.apply(
            &overlay,
            |key| config.borrow().get(key).cloned(),
            |key, value| {
                config
                    .borrow_mut()
                    .insert(key.to_string(), value.to_string());
            },
        );

        assert!(mgr.is_active());
        assert_eq!(config.borrow().get("model").unwrap(), "claude-3-opus");
        assert_eq!(config.borrow().get("model_provider").unwrap(), "anthropic");
    }

    #[test]
    fn test_restore_overlay() {
        let mut mgr = SessionModelManager::new();
        let config: RefCell<HashMap<String, String>> =
            RefCell::new(HashMap::from([("model".into(), "gpt-4".into())]));

        let overlay: SessionOverlay = HashMap::from([("model".into(), "claude-3-opus".into())]);

        mgr.apply(
            &overlay,
            |key| config.borrow().get(key).cloned(),
            |key, value| {
                config
                    .borrow_mut()
                    .insert(key.to_string(), value.to_string());
            },
        );

        assert_eq!(config.borrow().get("model").unwrap(), "claude-3-opus");

        mgr.restore(|key, value| {
            config
                .borrow_mut()
                .insert(key.to_string(), value.to_string());
        });

        assert!(!mgr.is_active());
        assert_eq!(config.borrow().get("model").unwrap(), "gpt-4");
    }

    #[test]
    fn test_apply_empty_overlay_is_noop() {
        let mut mgr = SessionModelManager::new();
        mgr.apply(&HashMap::new(), |_| None, |_, _| {});
        assert!(!mgr.is_active());
    }

    #[test]
    fn test_apply_ignores_invalid_fields() {
        let mut mgr = SessionModelManager::new();
        let config: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());

        let overlay: SessionOverlay = HashMap::from([("invalid_field".into(), "value".into())]);

        mgr.apply(
            &overlay,
            |key| config.borrow().get(key).cloned(),
            |key, value| {
                config
                    .borrow_mut()
                    .insert(key.to_string(), value.to_string());
            },
        );

        // overlay is set but no config was changed
        assert!(mgr.is_active());
        assert!(config.borrow().is_empty());
    }

    #[test]
    fn test_get_set_clear_session_model() {
        let mut metadata = serde_json::json!({});

        // Set
        let overlay: SessionOverlay = HashMap::from([("model".into(), "claude-3-opus".into())]);
        set_session_model(&mut metadata, &overlay);

        // Get
        let retrieved = get_session_model(&metadata);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().get("model").unwrap(), "claude-3-opus");

        // Clear
        clear_session_model(&mut metadata);
        assert!(get_session_model(&metadata).is_none());
    }

    #[test]
    fn test_validate_session_model() {
        let overlay: SessionOverlay = HashMap::from([
            ("model".into(), "gpt-4".into()),
            ("model_provider".into(), "openai".into()),
            ("garbage_field".into(), "value".into()),
        ]);

        let (valid, warnings) = validate_session_model(&overlay);
        assert_eq!(valid.len(), 2);
        assert!(valid.contains_key("model"));
        assert!(valid.contains_key("model_provider"));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("garbage_field"));
    }

    #[test]
    fn test_validate_empty() {
        let (valid, warnings) = validate_session_model(&HashMap::new());
        assert!(valid.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_session_model_fields_contains_expected() {
        assert!(SESSION_MODEL_FIELDS.contains(&"model"));
        assert!(SESSION_MODEL_FIELDS.contains(&"model_thinking"));
        assert!(SESSION_MODEL_FIELDS.contains(&"model_vlm_provider"));
    }
}
