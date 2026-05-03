// ============================================================
// PARTIE 4 — Persistance de l'historique d'exécutions
// ============================================================
// - Reçoit les événements via canal mpsc (crossbeam)
// - Stocke dans des fichiers JSON (un par tâche)
// - Rotation mensuelle des fichiers
// - Requêtes : dernière exécution, taux de succès
// ============================================================

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::thread;
use chrono::{DateTime, Local};
use crossbeam_channel::Receiver;
use serde::{Deserialize, Serialize};
use log::{info, error};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("Erreur I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Erreur JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Tâche inconnue: {0}")]
    UnknownTask(String),
}

/// Événement d'exécution envoyé par le moteur d'exécution (Partie 3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub task_name: String,
    pub started_at: DateTime<Local>,
    pub finished_at: DateTime<Local>,
    pub duration_secs: f64,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub timed_out: bool,
    pub attempt: u32,
}

/// Statistiques simples pour une tâche
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStats {
    pub task_name: String,
    pub total_runs: u64,
    pub successes: u64,
    pub failures: u64,
    pub timeouts: u64,
    pub success_rate: f64,
    pub last_run: Option<DateTime<Local>>,
    pub last_success: Option<DateTime<Local>>,
    pub last_failure: Option<DateTime<Local>>,
    pub avg_duration_secs: f64,
}

/// Gestionnaire d'historique
pub struct HistoryManager {
    /// Répertoire de stockage des journaux
    base_dir: PathBuf,
}

impl HistoryManager {
    /// Crée un gestionnaire d'historique avec le répertoire spécifié
    pub fn new<P: AsRef<Path>>(base_dir: P) -> Self {
        let path = base_dir.as_ref().to_path_buf();
        fs::create_dir_all(&path).ok();
        HistoryManager { base_dir: path }
    }

    /// Chemin du fichier journal pour une tâche et un mois donné
    /// Format : base_dir/<task_name>/<YYYY-MM>.jsonl
    fn log_path(&self, task_name: &str, date: &DateTime<Local>) -> PathBuf {
        let month_str = date.format("%Y-%m").to_string();
        let task_dir = self.base_dir.join(task_name);
        fs::create_dir_all(&task_dir).ok();
        task_dir.join(format!("{}.jsonl", month_str))
    }

    /// Enregistre un événement d'exécution dans le journal (append)
    pub fn record(&self, event: &ExecutionEvent) -> Result<(), HistoryError> {
        let path = self.log_path(&event.task_name, &event.started_at);
        let line = serde_json::to_string(event)?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        writeln!(file, "{}", line)?;
        info!(
            "[historique] Enregistré: {} — succès={} durée={:.2}s",
            event.task_name, event.success, event.duration_secs
        );
        Ok(())
    }

    /// Lit tous les événements pour une tâche (tous les mois disponibles)
    pub fn read_all(&self, task_name: &str) -> Result<Vec<ExecutionEvent>, HistoryError> {
        let task_dir = self.base_dir.join(task_name);
        if !task_dir.exists() {
            return Ok(vec![]);
        }

        let mut events = Vec::new();
        let entries = fs::read_dir(&task_dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                let events_in_file = self.read_file(&path)?;
                events.extend(events_in_file);
            }
        }

        // Trier par date de début
        events.sort_by_key(|e| e.started_at);
        Ok(events)
    }

    /// Lit les événements d'un fichier .jsonl
    fn read_file(&self, path: &PathBuf) -> Result<Vec<ExecutionEvent>, HistoryError> {
        let file = fs::File::open(path)?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ExecutionEvent>(&line) {
                Ok(event) => events.push(event),
                Err(e) => {
                    error!("Ligne JSON invalide dans {:?}: {}", path, e);
                }
            }
        }
        Ok(events)
    }

    /// Retourne la dernière exécution d'une tâche
    pub fn last_execution(&self, task_name: &str) -> Result<Option<ExecutionEvent>, HistoryError> {
        let events = self.read_all(task_name)?;
        Ok(events.into_iter().last())
    }

    /// Calcule les statistiques d'une tâche
    pub fn stats(&self, task_name: &str) -> Result<TaskStats, HistoryError> {
        let events = self.read_all(task_name)?;

        let total_runs = events.len() as u64;
        let successes = events.iter().filter(|e| e.success).count() as u64;
        let failures = events.iter().filter(|e| !e.success).count() as u64;
        let timeouts = events.iter().filter(|e| e.timed_out).count() as u64;

        let success_rate = if total_runs > 0 {
            successes as f64 / total_runs as f64 * 100.0
        } else {
            0.0
        };

        let avg_duration_secs = if total_runs > 0 {
            events.iter().map(|e| e.duration_secs).sum::<f64>() / total_runs as f64
        } else {
            0.0
        };

        let last_run = events.iter().map(|e| e.started_at).max();
        let last_success = events.iter().filter(|e| e.success).map(|e| e.started_at).max();
        let last_failure = events.iter().filter(|e| !e.success).map(|e| e.started_at).max();

        Ok(TaskStats {
            task_name: task_name.to_string(),
            total_runs,
            successes,
            failures,
            timeouts,
            success_rate,
            last_run,
            last_success,
            last_failure,
            avg_duration_secs,
        })
    }

    /// Retourne les statistiques de toutes les tâches dans le répertoire
    pub fn all_stats(&self) -> Result<Vec<TaskStats>, HistoryError> {
        let mut stats_list = Vec::new();

        if !self.base_dir.exists() {
            return Ok(vec![]);
        }

        for entry in fs::read_dir(&self.base_dir)?.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    let stats = self.stats(name)?;
                    stats_list.push(stats);
                }
            }
        }

        stats_list.sort_by(|a, b| a.task_name.cmp(&b.task_name));
        Ok(stats_list)
    }

    /// Lance un thread qui écoute le canal et enregistre les événements
    pub fn start_listener(self, rx: Receiver<ExecutionEvent>) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            info!("Gestionnaire d'historique démarré");
            for event in rx {
                if let Err(e) = self.record(&event) {
                    error!("Erreur d'enregistrement: {}", e);
                }
            }
            info!("Gestionnaire d'historique arrêté");
        })
    }
}

// ============================================================
// TESTS — Partie 4
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;
    use crossbeam_channel::unbounded;

    fn make_event(name: &str, success: bool, duration: f64) -> ExecutionEvent {
        let now = Local::now();
        ExecutionEvent {
            task_name: name.to_string(),
            started_at: now,
            finished_at: now + chrono::Duration::seconds(duration as i64),
            duration_secs: duration,
            exit_code: if success { Some(0) } else { Some(1) },
            stdout: "sortie test".into(),
            stderr: if success { String::new() } else { "erreur".into() },
            success,
            timed_out: false,
            attempt: 0,
        }
    }

    #[test]
    fn test_enregistrement_et_lecture() {
        let dir = TempDir::new().unwrap();
        let manager = HistoryManager::new(dir.path());

        let event = make_event("ma_tache", true, 1.5);
        manager.record(&event).unwrap();

        let events = manager.read_all("ma_tache").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].task_name, "ma_tache");
        assert!(events[0].success);
    }

    #[test]
    fn test_plusieurs_enregistrements() {
        let dir = TempDir::new().unwrap();
        let manager = HistoryManager::new(dir.path());

        for i in 0..5 {
            let event = make_event("tache_multi", i % 2 == 0, 2.0);
            manager.record(&event).unwrap();
        }

        let events = manager.read_all("tache_multi").unwrap();
        assert_eq!(events.len(), 5);
    }

    #[test]
    fn test_tache_inexistante() {
        let dir = TempDir::new().unwrap();
        let manager = HistoryManager::new(dir.path());
        let events = manager.read_all("inconnue").unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_derniere_execution() {
        let dir = TempDir::new().unwrap();
        let manager = HistoryManager::new(dir.path());

        for _ in 0..3 {
            let event = make_event("task_last", true, 1.0);
            manager.record(&event).unwrap();
        }

        let last = manager.last_execution("task_last").unwrap();
        assert!(last.is_some());
    }

    #[test]
    fn test_stats_succes() {
        let dir = TempDir::new().unwrap();
        let manager = HistoryManager::new(dir.path());

        // 3 succès, 1 échec
        for i in 0..4 {
            let event = make_event("task_stats", i < 3, 1.0);
            manager.record(&event).unwrap();
        }

        let stats = manager.stats("task_stats").unwrap();
        assert_eq!(stats.total_runs, 4);
        assert_eq!(stats.successes, 3);
        assert_eq!(stats.failures, 1);
        assert!((stats.success_rate - 75.0).abs() < 0.01);
    }

    #[test]
    fn test_stats_taux_zero() {
        let dir = TempDir::new().unwrap();
        let manager = HistoryManager::new(dir.path());
        let stats = manager.stats("vide").unwrap();
        assert_eq!(stats.total_runs, 0);
        assert_eq!(stats.success_rate, 0.0);
    }

    #[test]
    fn test_stats_moyenne_duree() {
        let dir = TempDir::new().unwrap();
        let manager = HistoryManager::new(dir.path());

        manager.record(&make_event("avg", true, 2.0)).unwrap();
        manager.record(&make_event("avg", true, 4.0)).unwrap();

        let stats = manager.stats("avg").unwrap();
        assert!((stats.avg_duration_secs - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_listener_thread() {
        let dir = TempDir::new().unwrap();
        let manager = HistoryManager::new(dir.path());
        let (tx, rx) = unbounded();

        let handle = manager.start_listener(rx);

        // Envoyer des événements
        for i in 0..3 {
            tx.send(make_event("thread_task", i % 2 == 0, 0.5)).unwrap();
        }
        drop(tx); // Fermer le canal pour arrêter le thread

        handle.join().unwrap();

        // Vérifier que les événements ont été enregistrés
        let base = dir.path();
        let manager2 = HistoryManager::new(base);
        let events = manager2.read_all("thread_task").unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn test_serialisation_roundtrip() {
        let event = make_event("roundtrip", true, 1.23);
        let json = serde_json::to_string(&event).unwrap();
        let back: ExecutionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.task_name, event.task_name);
        assert_eq!(back.success, event.success);
        assert!((back.duration_secs - event.duration_secs).abs() < 0.001);
    }
}
