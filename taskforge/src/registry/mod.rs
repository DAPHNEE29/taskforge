mod task_ext;

// ============================================================
// PARTIE 2 — Registre de tâches et configuration TOML
// ============================================================

use std::collections::HashMap;
use std::path::Path;
use std::fs;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use crate::scheduler::Schedule;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("Erreur de lecture du fichier: {0}")]
    Io(#[from] std::io::Error),
    #[error("Erreur de parsing TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Tâche inconnue: {0}")]
    UnknownTask(String),
    #[error("Tâche dupliquée: {0}")]
    DuplicateTask(String),
    #[error("Expression de planification invalide pour '{name}': {err}")]
    InvalidSchedule { name: String, err: String },
}

/// Type de retry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RetryKind {
    #[default]
    None,
    Immediate,
    Fixed,
    Exponential,
}

/// Politique de retry en cas d'échec (format plat, compatible TOML)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryPolicy {
    /// Type : "none", "immediate", "fixed", "exponential"
    #[serde(default)]
    pub kind: RetryKind,
    /// Nombre maximum de retries
    #[serde(default)]
    pub max_retries: u32,
    /// Délai en secondes (pour Fixed)
    #[serde(default)]
    pub delay_secs: u64,
    /// Délai initial en secondes (pour Exponential)
    #[serde(default)]
    pub initial_delay_secs: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            kind: RetryKind::None,
            max_retries: 0,
            delay_secs: 0,
            initial_delay_secs: 0,
        }
    }
}

/// Configuration brute lue depuis le TOML
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    pub name: String,
    pub command: String,
    pub schedule: String,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool { true }

#[derive(Debug, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    tasks: Vec<TaskConfig>,
}

/// Tâche avec planification parsée
#[derive(Debug, Clone)]
pub struct Task {
    pub config: TaskConfig,
    pub schedule: Schedule,
}

impl Task {
    pub fn from_config(config: TaskConfig) -> Result<Self, RegistryError> {
        let schedule = Schedule::parse(&config.schedule).map_err(|e| {
            RegistryError::InvalidSchedule {
                name: config.name.clone(),
                err: e.to_string(),
            }
        })?;
        Ok(Task { config, schedule })
    }

    pub fn name(&self) -> &str { &self.config.name }
    pub fn command(&self) -> &str { &self.config.command }
    pub fn is_enabled(&self) -> bool { self.config.enabled }

    pub fn timeout(&self) -> Option<std::time::Duration> {
        self.config.timeout_secs.map(std::time::Duration::from_secs)
    }

    pub fn retry_policy(&self) -> &RetryPolicy { &self.config.retry }
}

/// Registre de toutes les tâches
#[derive(Debug)]
pub struct TaskRegistry {
    tasks: HashMap<String, Task>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        TaskRegistry { tasks: HashMap::new() }
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, RegistryError> {
        let content = fs::read_to_string(&path)?;
        Self::load_from_str(&content)
    }

    pub fn load_from_str(content: &str) -> Result<Self, RegistryError> {
        let config_file: ConfigFile = toml::from_str(content)?;
        let mut registry = TaskRegistry::new();
        for task_config in config_file.tasks {
            registry.add_task(task_config)?;
        }
        Ok(registry)
    }

    pub fn add_task(&mut self, config: TaskConfig) -> Result<(), RegistryError> {
        let name = config.name.clone();
        if self.tasks.contains_key(&name) {
            return Err(RegistryError::DuplicateTask(name));
        }
        let task = Task::from_config(config)?;
        self.tasks.insert(name, task);
        Ok(())
    }

    pub fn enabled_tasks(&self) -> Vec<&Task> {
        self.tasks.values().filter(|t| t.is_enabled()).collect()
    }

    pub fn all_tasks(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks.values().collect();
        tasks.sort_by_key(|t| t.name());
        tasks
    }

    pub fn get_task(&self, name: &str) -> Result<&Task, RegistryError> {
        self.tasks.get(name).ok_or_else(|| RegistryError::UnknownTask(name.to_string()))
    }

    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<(), RegistryError> {
        let task = self.tasks.get_mut(name)
            .ok_or_else(|| RegistryError::UnknownTask(name.to_string()))?;
        task.config.enabled = enabled;
        Ok(())
    }

    pub fn len(&self) -> usize { self.tasks.len() }
    pub fn is_empty(&self) -> bool { self.tasks.is_empty() }
}

// ============================================================
// TESTS — Partie 2
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;

    fn config_toml_valide() -> &'static str {
        r#"
[[tasks]]
name = "backup_db"
command = "pg_dump mydb > /tmp/backup.sql"
schedule = "@daily"
timeout_secs = 300
description = "Sauvegarde nocturne"
enabled = true

[tasks.retry]
kind = "fixed"
max_retries = 3
delay_secs = 5

[[tasks]]
name = "nettoyage_tmp"
command = "find /tmp -mtime +7 -delete"
schedule = "0 3 * * 0"
enabled = true

[[tasks]]
name = "rapport_hebdo"
command = "python3 /opt/rapports/generer.py"
schedule = "@weekly"
enabled = false
"#
    }

    #[test]
    fn test_chargement_toml() {
        let registry = TaskRegistry::load_from_str(config_toml_valide()).unwrap();
        assert_eq!(registry.len(), 3);
    }

    #[test]
    fn test_taches_actives() {
        let registry = TaskRegistry::load_from_str(config_toml_valide()).unwrap();
        assert_eq!(registry.enabled_tasks().len(), 2);
    }

    #[test]
    fn test_get_task() {
        let registry = TaskRegistry::load_from_str(config_toml_valide()).unwrap();
        let task = registry.get_task("nettoyage_tmp").unwrap();
        assert_eq!(task.command(), "find /tmp -mtime +7 -delete");
    }

    #[test]
    fn test_task_inconnue() {
        let registry = TaskRegistry::load_from_str(config_toml_valide()).unwrap();
        assert!(registry.get_task("inexistante").is_err());
    }

    #[test]
    fn test_tache_dupliquee() {
        let toml = r#"
[[tasks]]
name = "doublon"
command = "echo 1"
schedule = "@hourly"

[[tasks]]
name = "doublon"
command = "echo 2"
schedule = "@daily"
"#;
        let result = TaskRegistry::load_from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_schedule_invalide() {
        let toml = r#"
[[tasks]]
name = "mauvaise"
command = "echo test"
schedule = "invalide"
"#;
        let result = TaskRegistry::load_from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_activer_desactiver() {
        let mut registry = TaskRegistry::load_from_str(config_toml_valide()).unwrap();
        registry.set_enabled("rapport_hebdo", true).unwrap();
        assert_eq!(registry.enabled_tasks().len(), 3);
        registry.set_enabled("backup_db", false).unwrap();
        assert_eq!(registry.enabled_tasks().len(), 2);
    }

    #[test]
    fn test_timeout() {
        let registry = TaskRegistry::load_from_str(config_toml_valide()).unwrap();
        let task = registry.get_task("backup_db").unwrap();
        assert_eq!(task.timeout(), Some(std::time::Duration::from_secs(300)));
    }

    #[test]
    fn test_sans_timeout() {
        let registry = TaskRegistry::load_from_str(config_toml_valide()).unwrap();
        let task = registry.get_task("nettoyage_tmp").unwrap();
        assert_eq!(task.timeout(), None);
    }

    #[test]
    fn test_serialisation_retry_policy() {
        let policy = RetryPolicy {
            kind: RetryKind::Fixed,
            max_retries: 3,
            delay_secs: 5,
            initial_delay_secs: 0,
        };
        let json = serde_json::to_string(&policy).unwrap();
        let back: RetryPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, back);
    }

    #[test]
    fn test_registre_vide() {
        let registry = TaskRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.enabled_tasks().len(), 0);
    }
}
