//! Config version migration support.
//!
//! Adds a `config_version` field to config files and applies migration
//! functions when loading configs with older versions.

use tracing::{debug, info};

/// Current config version. Increment this when adding migrations.
pub const CURRENT_CONFIG_VERSION: u32 = 2;

/// Key used to store the config version in JSON.
pub const VERSION_KEY: &str = "config_version";

/// Migrate a JSON config value from its stored version to the current version.
///
/// Returns the migrated JSON value and whether any migrations were applied.
pub fn migrate_config(mut value: serde_json::Value) -> (serde_json::Value, bool) {
    let stored_version = value
        .as_object()
        .and_then(|obj| obj.get(VERSION_KEY))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(0);

    if stored_version >= CURRENT_CONFIG_VERSION {
        // Already at current version, just ensure version field is set
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                VERSION_KEY.to_string(),
                serde_json::Value::Number(CURRENT_CONFIG_VERSION.into()),
            );
        }
        return (value, false);
    }

    let mut version = stored_version;
    let mut migrated = false;

    // Apply migrations in order
    while version < CURRENT_CONFIG_VERSION {
        match version {
            0 => {
                // Migration from version 0 (no version field) to version 1:
                // - Add config_version field
                // - No structural changes needed for v1; this just stamps the version.
                value = migrate_v0_to_v1(value);
                version = 1;
                migrated = true;
                info!("Migrated config from v0 to v1");
            }
            1 => {
                // Migration from version 1 to version 2:
                // - Move model_compact/model_compact_provider into agents.compact
                // - Remove model_critique/model_critique_provider (dead code)
                value = migrate_v1_to_v2(value);
                version = 2;
                migrated = true;
                info!("Migrated config from v1 to v2");
            }
            _ => {
                // Unknown version — skip remaining migrations
                debug!(
                    "Unknown config version {}, skipping further migrations",
                    version
                );
                break;
            }
        }
    }

    (value, migrated)
}

/// Migrate from version 0 (unversioned) to version 1.
///
/// Version 1 simply stamps the config_version field. Future migrations
/// can add structural changes here.
fn migrate_v0_to_v1(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = value.as_object_mut() {
        obj.insert(VERSION_KEY.to_string(), serde_json::Value::Number(1.into()));
    }
    value
}

/// Migrate from version 1 to version 2.
///
/// - Moves `model_compact` / `model_compact_provider` into `agents.compact`
/// - Removes `model_critique` / `model_critique_provider` (dead code, never
///   consumed by any runtime path)
fn migrate_v1_to_v2(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = value.as_object_mut() {
        // Extract compact fields before removing them
        let compact_model = obj
            .remove("model_compact")
            .and_then(|v| v.as_str().map(String::from));
        let compact_provider = obj
            .remove("model_compact_provider")
            .and_then(|v| v.as_str().map(String::from));

        // Remove critique fields (dead code)
        obj.remove("model_critique");
        obj.remove("model_critique_provider");

        // If either compact field was set, write into agents.compact
        if compact_model.is_some() || compact_provider.is_some() {
            let agents = obj.entry("agents").or_insert_with(|| serde_json::json!({}));
            if let Some(agents_obj) = agents.as_object_mut() {
                let mut compact_entry = serde_json::Map::new();
                if let Some(model) = compact_model {
                    compact_entry.insert("model".to_string(), serde_json::Value::String(model));
                }
                if let Some(provider) = compact_provider {
                    compact_entry
                        .insert("provider".to_string(), serde_json::Value::String(provider));
                }
                agents_obj.insert(
                    "compact".to_string(),
                    serde_json::Value::Object(compact_entry),
                );
            }
        }

        // Stamp version
        obj.insert(VERSION_KEY.to_string(), serde_json::Value::Number(2.into()));
    }
    value
}

/// Check whether a config value needs migration.
pub fn needs_migration(value: &serde_json::Value) -> bool {
    let stored_version = value
        .as_object()
        .and_then(|obj| obj.get(VERSION_KEY))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(0);

    stored_version < CURRENT_CONFIG_VERSION
}

/// Get the version of a config value.
pub fn config_version(value: &serde_json::Value) -> u32 {
    value
        .as_object()
        .and_then(|obj| obj.get(VERSION_KEY))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_version() {
        assert!(CURRENT_CONFIG_VERSION >= 2);
    }

    #[test]
    fn test_migrate_unversioned_config() {
        let value = serde_json::json!({
            "model_provider": "openai",
            "model": "gpt-4"
        });

        assert!(needs_migration(&value));
        assert_eq!(config_version(&value), 0);

        let (migrated, changed) = migrate_config(value);
        assert!(changed);
        assert_eq!(config_version(&migrated), CURRENT_CONFIG_VERSION);

        // Original fields preserved
        assert_eq!(migrated["model_provider"], "openai");
        assert_eq!(migrated["model"], "gpt-4");
    }

    #[test]
    fn test_migrate_already_current() {
        let value = serde_json::json!({
            "config_version": CURRENT_CONFIG_VERSION,
            "model_provider": "anthropic",
            "model": "claude-3-opus"
        });

        assert!(!needs_migration(&value));

        let (migrated, changed) = migrate_config(value);
        assert!(!changed);
        assert_eq!(config_version(&migrated), CURRENT_CONFIG_VERSION);
    }

    #[test]
    fn test_migrate_v0_to_v1() {
        let value = serde_json::json!({
            "model_provider": "fireworks",
            "temperature": 0.7,
            "verbose": true
        });

        let migrated = migrate_v0_to_v1(value);
        assert_eq!(config_version(&migrated), 1);
        assert_eq!(migrated["model_provider"], "fireworks");
        assert_eq!(migrated["temperature"], 0.7);
        assert_eq!(migrated["verbose"], true);
    }

    #[test]
    fn test_migrate_v1_to_v2_with_compact() {
        let value = serde_json::json!({
            "config_version": 1,
            "model_provider": "openai",
            "model": "gpt-4",
            "model_compact": "gemini-2.0-flash",
            "model_compact_provider": "google"
        });

        let (migrated, changed) = migrate_config(value);
        assert!(changed);
        assert_eq!(config_version(&migrated), 2);

        // Flat fields removed
        assert!(migrated.get("model_compact").is_none());
        assert!(migrated.get("model_compact_provider").is_none());

        // Moved into agents.compact
        assert_eq!(migrated["agents"]["compact"]["model"], "gemini-2.0-flash");
        assert_eq!(migrated["agents"]["compact"]["provider"], "google");

        // Original fields preserved
        assert_eq!(migrated["model_provider"], "openai");
        assert_eq!(migrated["model"], "gpt-4");
    }

    #[test]
    fn test_migrate_v1_to_v2_removes_critique() {
        let value = serde_json::json!({
            "config_version": 1,
            "model_critique": "gpt-4o",
            "model_critique_provider": "openai"
        });

        let (migrated, _) = migrate_config(value);
        assert!(migrated.get("model_critique").is_none());
        assert!(migrated.get("model_critique_provider").is_none());
    }

    #[test]
    fn test_migrate_v1_to_v2_no_compact_no_agents() {
        let value = serde_json::json!({
            "config_version": 1,
            "model_provider": "openai",
            "model": "gpt-4"
        });

        let (migrated, changed) = migrate_config(value);
        assert!(changed);
        assert_eq!(config_version(&migrated), 2);
        // No agents entry created when no compact was set
        assert!(migrated.get("agents").is_none());
    }

    #[test]
    fn test_migrate_v1_to_v2_preserves_existing_agents() {
        let value = serde_json::json!({
            "config_version": 1,
            "model_compact": "flash",
            "model_compact_provider": "google",
            "agents": {
                "explore": { "model": "gpt-4o", "max_steps": 5 }
            }
        });

        let (migrated, _) = migrate_config(value);
        // Existing agent preserved
        assert_eq!(migrated["agents"]["explore"]["model"], "gpt-4o");
        assert_eq!(migrated["agents"]["explore"]["max_steps"], 5);
        // Compact added
        assert_eq!(migrated["agents"]["compact"]["model"], "flash");
        assert_eq!(migrated["agents"]["compact"]["provider"], "google");
    }

    #[test]
    fn test_migrate_v1_to_v2_compact_model_only() {
        // Only model set, no provider
        let value = serde_json::json!({
            "config_version": 1,
            "model_compact": "flash"
        });

        let (migrated, _) = migrate_config(value);
        assert_eq!(migrated["agents"]["compact"]["model"], "flash");
        // No provider key
        assert!(migrated["agents"]["compact"].get("provider").is_none());
    }

    #[test]
    fn test_migrate_v0_through_v2() {
        // Full migration from v0 through all versions
        let value = serde_json::json!({
            "model_provider": "openai",
            "model": "gpt-4",
            "model_compact": "flash",
            "model_compact_provider": "google",
            "model_critique": "gpt-4o",
            "model_critique_provider": "openai"
        });

        assert_eq!(config_version(&value), 0);
        let (migrated, changed) = migrate_config(value);
        assert!(changed);
        assert_eq!(config_version(&migrated), 2);

        // Critique removed
        assert!(migrated.get("model_critique").is_none());
        assert!(migrated.get("model_critique_provider").is_none());

        // Compact moved to agents
        assert!(migrated.get("model_compact").is_none());
        assert_eq!(migrated["agents"]["compact"]["model"], "flash");
        assert_eq!(migrated["agents"]["compact"]["provider"], "google");
    }

    #[test]
    fn test_needs_migration_empty_object() {
        let value = serde_json::json!({});
        assert!(needs_migration(&value));
    }

    #[test]
    fn test_config_version_missing() {
        let value = serde_json::json!({"model": "gpt-4"});
        assert_eq!(config_version(&value), 0);
    }

    #[test]
    fn test_config_version_present() {
        let value = serde_json::json!({"config_version": 2});
        assert_eq!(config_version(&value), 2);
    }

    #[test]
    fn test_migrate_preserves_all_fields() {
        let value = serde_json::json!({
            "model_provider": "openai",
            "model": "gpt-4",
            "api_key": "sk-test",
            "max_tokens": 8192,
            "temperature": 0.5,
            "verbose": true,
            "debug_logging": false,
            "custom_field": "should_survive"
        });

        let (migrated, _) = migrate_config(value);
        assert_eq!(migrated["model_provider"], "openai");
        assert_eq!(migrated["api_key"], "sk-test");
        assert_eq!(migrated["max_tokens"], 8192);
        assert_eq!(migrated["custom_field"], "should_survive");
    }
}
