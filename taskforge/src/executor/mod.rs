// ============================================================
// PARTIE 3 — Moteur d'exécution avec supervision
// ============================================================

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::thread;
use chrono::Local;
use crossbeam_channel::Sender;
use log::{info, warn, error};

use crate::registry::{Task, TaskRegistry, RetryPolicy, RetryKind};
use crate::history::ExecutionEvent;

/// Résultat d'une exécution de tâche
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub task_name: String,
    pub started_at: chrono::DateTime<Local>,
    pub finished_at: chrono::DateTime<Local>,
    pub duration: Duration,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub timed_out: bool,
}

/// Retourne le bon interpréteur shell selon l'OS
fn shell_command(command: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", command]);
        cmd
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);
        cmd
    }
}

/// Exécute une commande shell et retourne le résultat
pub fn run_command(
    task_name: &str,
    command: &str,
    timeout: Option<Duration>,
) -> ExecutionResult {
    let started_at = Local::now();
    let start = Instant::now();

    info!("[{}] Démarrage : {}", task_name, command);

    let child_result = shell_command(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    match child_result {
        Err(e) => {
            let duration = start.elapsed();
            error!("[{}] Impossible de lancer la commande: {}", task_name, e);
            ExecutionResult {
                task_name: task_name.to_string(),
                started_at,
                finished_at: Local::now(),
                duration,
                exit_code: None,
                stdout: String::new(),
                stderr: format!("Erreur de lancement: {}", e),
                success: false,
                timed_out: false,
            }
        }
        Ok(mut child) => {
            match timeout {
                Some(timeout_dur) => {
                    let deadline = Instant::now() + timeout_dur;
                    loop {
                        match child.try_wait() {
                            Ok(Some(_)) => break,
                            Ok(None) => {
                                if Instant::now() >= deadline {
                                    let _ = child.kill();
                                    let _ = child.wait();
                                    let duration = start.elapsed();
                                    warn!("[{}] Processus tué après timeout", task_name);
                                    return ExecutionResult {
                                        task_name: task_name.to_string(),
                                        started_at,
                                        finished_at: Local::now(),
                                        duration,
                                        exit_code: None,
                                        stdout: String::new(),
                                        stderr: format!("Timeout de {:?} dépassé", timeout_dur),
                                        success: false,
                                        timed_out: true,
                                    };
                                }
                                thread::sleep(Duration::from_millis(100));
                            }
                            Err(e) => {
                                error!("[{}] Erreur wait: {}", task_name, e);
                                break;
                            }
                        }
                    }
                    match child.wait_with_output() {
                        Ok(out) => {
                            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                            let success = out.status.code() == Some(0);
                            let duration = start.elapsed();
                            if success { info!("[{}] Succès en {:?}", task_name, duration); }
                            else { warn!("[{}] Échec en {:?}", task_name, duration); }
                            ExecutionResult {
                                task_name: task_name.to_string(),
                                started_at,
                                finished_at: Local::now(),
                                duration,
                                exit_code: out.status.code(),
                                stdout, stderr, success,
                                timed_out: false,
                            }
                        }
                        Err(e) => ExecutionResult {
                            task_name: task_name.to_string(),
                            started_at,
                            finished_at: Local::now(),
                            duration: start.elapsed(),
                            exit_code: None,
                            stdout: String::new(),
                            stderr: e.to_string(),
                            success: false,
                            timed_out: false,
                        },
                    }
                }
                None => {
                    match child.wait_with_output() {
                        Ok(out) => {
                            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                            let success = out.status.code() == Some(0);
                            let duration = start.elapsed();
                            if success { info!("[{}] Succès en {:?}", task_name, duration); }
                            else { warn!("[{}] Échec en {:?}", task_name, duration); }
                            ExecutionResult {
                                task_name: task_name.to_string(),
                                started_at,
                                finished_at: Local::now(),
                                duration,
                                exit_code: out.status.code(),
                                stdout, stderr, success,
                                timed_out: false,
                            }
                        }
                        Err(e) => ExecutionResult {
                            task_name: task_name.to_string(),
                            started_at,
                            finished_at: Local::now(),
                            duration: start.elapsed(),
                            exit_code: None,
                            stdout: String::new(),
                            stderr: e.to_string(),
                            success: false,
                            timed_out: false,
                        },
                    }
                }
            }
        }
    }
}

/// Exécute une tâche avec sa politique de retry
pub fn run_with_retry(task: &Task, event_tx: &Sender<ExecutionEvent>) {
    let policy = task.retry_policy().clone();
    let max_retries = policy.max_retries;

    let mut attempt = 0u32;
    loop {
        let result = run_command(task.name(), task.command(), task.timeout());
        let is_success = result.success;

        let event = ExecutionEvent {
            task_name: result.task_name.clone(),
            started_at: result.started_at,
            finished_at: result.finished_at,
            duration_secs: result.duration.as_secs_f64(),
            exit_code: result.exit_code,
            stdout: result.stdout.clone(),
            stderr: result.stderr.clone(),
            success: result.success,
            timed_out: result.timed_out,
            attempt,
        };
        let _ = event_tx.send(event);

        if is_success || attempt >= max_retries || policy.kind == RetryKind::None {
            break;
        }

        attempt += 1;
        warn!("[{}] Échec — retry {}/{}", task.name(), attempt, max_retries);

        let delay = match policy.kind {
            RetryKind::None => break,
            RetryKind::Immediate => Duration::from_millis(0),
            RetryKind::Fixed => Duration::from_secs(policy.delay_secs),
            RetryKind::Exponential => {
                Duration::from_secs(policy.initial_delay_secs * 2u64.pow(attempt - 1))
            }
        };

        if delay.as_millis() > 0 {
            info!("[{}] Attente {:?} avant retry", task.name(), delay);
            thread::sleep(delay);
        }
    }
}

/// Moteur de supervision principal
pub struct Supervisor {
    registry: Arc<Mutex<TaskRegistry>>,
    event_tx: Sender<ExecutionEvent>,
    next_runs: std::collections::HashMap<String, chrono::DateTime<Local>>,
    running: Arc<Mutex<bool>>,
}

impl Supervisor {
    pub fn new(registry: Arc<Mutex<TaskRegistry>>, event_tx: Sender<ExecutionEvent>) -> Self {
        Supervisor {
            registry,
            event_tx,
            next_runs: std::collections::HashMap::new(),
            running: Arc::new(Mutex::new(false)),
        }
    }

    fn init_next_runs(&mut self) {
        let now = Local::now();
        let registry = self.registry.lock().unwrap();
        for task in registry.enabled_tasks() {
            if let Some(next) = task.schedule.next_occurrence(now) {
                info!("[{}] Prochaine exécution : {}", task.name(), next.format("%Y-%m-%d %H:%M:%S"));
                self.next_runs.insert(task.name().to_string(), next);
            }
        }
    }

    pub fn run(&mut self) {
        self.init_next_runs();
        *self.running.lock().unwrap() = true;
        info!("Superviseur démarré — {} tâche(s) surveillée(s)", self.next_runs.len());

        loop {
            if !*self.running.lock().unwrap() { break; }

            let now = Local::now();
            let mut to_run: Vec<String> = Vec::new();

            for (name, next_time) in &self.next_runs {
                if *next_time <= now {
                    to_run.push(name.clone());
                }
            }

            for task_name in to_run {
                let registry = self.registry.lock().unwrap();
                if let Ok(task) = registry.get_task(&task_name) {
                    if let Some(next) = task.schedule.next_occurrence(now) {
                        self.next_runs.insert(task_name.clone(), next);
                    }
                    let task_config = task.config.clone();
                    let schedule_clone = task.schedule.clone();
                    let event_tx = self.event_tx.clone();

                    info!("[{}] Lancement dans un thread séparé", task_name);
                    thread::spawn(move || {
                        let task = Task::from_config_with_schedule(task_config, schedule_clone);
                        run_with_retry(&task, &event_tx);
                    });
                }
            }

            thread::sleep(Duration::from_secs(1));
        }
        info!("Superviseur arrêté.");
    }

    pub fn stop(&self) {
        *self.running.lock().unwrap() = false;
    }
}

// ============================================================
// TESTS — Partie 3 (compatibles Windows et Linux/macOS)
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use crate::registry::{TaskConfig, RetryPolicy, RetryKind};

    /// Commande echo compatible Windows et Unix
    fn cmd_echo(msg: &str) -> String {
        format!("echo {}", msg)
    }

    /// Commande qui échoue (code 1)
    fn cmd_fail() -> &'static str {
        if cfg!(target_os = "windows") { "exit /b 1" } else { "exit 1" }
    }

    /// Commande qui prend longtemps (pour tester le timeout)
    fn cmd_sleep_long() -> &'static str {
        if cfg!(target_os = "windows") { "ping -n 30 127.0.0.1 > nul" } else { "sleep 10" }
    }

    /// Commande stderr
    fn cmd_stderr() -> &'static str {
        if cfg!(target_os = "windows") {
            "echo erreur 1>&2"
        } else {
            "echo erreur >&2"
        }
    }

    /// Commande multilignes
    fn cmd_multilines() -> &'static str {
        if cfg!(target_os = "windows") {
            "echo ligne1 && echo ligne2 && echo ligne3"
        } else {
            "printf 'ligne1\\nligne2\\nligne3\\n'"
        }
    }

    #[test]
    fn test_commande_simple_succes() {
        let result = run_command("test_echo", &cmd_echo("hello"), None);
        assert!(result.success, "La commande devrait réussir");
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("hello"));
        assert!(!result.timed_out);
    }

    #[test]
    fn test_commande_echec() {
        let result = run_command("test_fail", cmd_fail(), None);
        assert!(!result.success, "La commande devrait échouer");
        assert_ne!(result.exit_code, Some(0));
    }

    #[test]
    fn test_commande_stderr() {
        let result = run_command("test_stderr", cmd_stderr(), None);
        // La commande réussit mais écrit sur stderr
        assert!(result.stderr.contains("erreur") || result.stdout.contains("erreur"),
            "stdout='{}' stderr='{}'", result.stdout, result.stderr);
    }

    #[test]
    fn test_timeout_respecte() {
        let result = run_command(
            "test_timeout",
            cmd_sleep_long(),
            Some(Duration::from_millis(500)),
        );
        assert!(!result.success, "La commande devrait être tuée");
        assert!(result.timed_out, "timed_out devrait être true");
    }

    #[test]
    fn test_capture_stdout_multilignes() {
        let result = run_command("test_multiline", cmd_multilines(), None);
        assert!(result.success);
        assert!(result.stdout.contains("ligne1"));
        assert!(result.stdout.contains("ligne2"));
    }

    #[test]
    fn test_evenement_envoye() {
        let (tx, rx) = unbounded();
        let config = TaskConfig {
            name: "test_event".into(),
            command: cmd_echo("ok"),
            schedule: "@hourly".into(),
            timeout_secs: None,
            retry: RetryPolicy::default(),
            description: None,
            enabled: true,
        };
        let task = Task::from_config(config).unwrap();
        run_with_retry(&task, &tx);

        let event = rx.try_recv().expect("Un événement doit être envoyé");
        assert_eq!(event.task_name, "test_event");
        assert!(event.success);
        assert_eq!(event.attempt, 0);
    }

    #[test]
    fn test_retry_immediate() {
        let (tx, rx) = unbounded::<crate::history::ExecutionEvent>();
        let config = TaskConfig {
            name: "test_retry".into(),
            command: cmd_fail().to_string(),
            schedule: "@hourly".into(),
            timeout_secs: None,
            retry: RetryPolicy {
                kind: RetryKind::Immediate,
                max_retries: 2,
                delay_secs: 0,
                initial_delay_secs: 0,
            },
            description: None,
            enabled: true,
        };
        let task = Task::from_config(config).unwrap();
        run_with_retry(&task, &tx);

        // 1 exécution initiale + 2 retries = 3 événements
        let events: Vec<_> = rx.try_iter().collect();
        assert_eq!(events.len(), 3, "Devrait avoir 3 événements (1 + 2 retries)");
        assert_eq!(events[0].attempt, 0);
        assert_eq!(events[1].attempt, 1);
        assert_eq!(events[2].attempt, 2);
    }
}
