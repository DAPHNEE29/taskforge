// ============================================================
// PARTIE 5 — CLI de gestion et rapport de santé
// ============================================================
// Commandes :
//   taskforge list                     — lister les tâches
//   taskforge next [nom]               — prochaine exécution
//   taskforge run <nom>                — forcer une exécution
//   taskforge enable <nom>             — activer une tâche
//   taskforge disable <nom>            — désactiver une tâche
//   taskforge health                   — rapport de santé complet
//   taskforge daemon                   — lancer le daemon
// ============================================================

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use chrono::Local;

use crate::registry::TaskRegistry;
use crate::history::HistoryManager;
use crate::executor::run_with_retry;
use crate::history::ExecutionEvent;
use crossbeam_channel::unbounded;

/// TaskForge — Planificateur de tâches système (cron-like) en Rust
#[derive(Parser, Debug)]
#[command(
    name = "taskforge",
    about = "Planificateur de tâches système moderne, écrit en Rust",
    version = "0.1.0",
    author = "ENSPD"
)]
pub struct Cli {
    /// Chemin vers le fichier de configuration TOML
    #[arg(short, long, default_value = "config/tasks.toml")]
    pub config: PathBuf,

    /// Répertoire de stockage de l'historique
    #[arg(short = 'H', long, default_value = "history")]
    pub history_dir: PathBuf,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Lister toutes les tâches configurées
    List {
        /// Afficher seulement les tâches actives
        #[arg(short, long)]
        enabled_only: bool,
    },

    /// Afficher la prochaine exécution d'une ou toutes les tâches
    Next {
        /// Nom de la tâche (optionnel, toutes si absent)
        name: Option<String>,
    },

    /// Forcer l'exécution immédiate d'une tâche
    Run {
        /// Nom de la tâche à exécuter
        name: String,
    },

    /// Activer une tâche
    Enable {
        /// Nom de la tâche
        name: String,
    },

    /// Désactiver une tâche sans redémarrage du daemon
    Disable {
        /// Nom de la tâche
        name: String,
    },

    /// Afficher le rapport de santé du planificateur
    Health,

    /// Lancer le daemon de planification
    Daemon,
}

/// Exécute la commande CLI avec le registre et le gestionnaire d'historique donnés
pub fn run_command(
    cli: &Cli,
    registry: &mut TaskRegistry,
    history: &HistoryManager,
) {
    match &cli.command {
        Commands::List { enabled_only } => {
            cmd_list(registry, *enabled_only);
        }
        Commands::Next { name } => {
            cmd_next(registry, name.as_deref());
        }
        Commands::Run { name } => {
            cmd_run(registry, history, name);
        }
        Commands::Enable { name } => {
            cmd_set_enabled(registry, name, true);
        }
        Commands::Disable { name } => {
            cmd_set_enabled(registry, name, false);
        }
        Commands::Health => {
            cmd_health(registry, history);
        }
        Commands::Daemon => {
            cmd_daemon(registry, history, &cli.history_dir);
        }
    }
}

// ---- Implémentations des commandes ----

fn cmd_list(registry: &TaskRegistry, enabled_only: bool) {
    let tasks = if enabled_only {
        registry.enabled_tasks()
    } else {
        registry.all_tasks()
    };

    if tasks.is_empty() {
        println!("Aucune tâche configurée.");
        return;
    }

    println!("\n{:<20} {:<10} {:<35} {}", "NOM", "STATUT", "PLANIFICATION", "COMMANDE");
    println!("{}", "-".repeat(90));

    for task in tasks {
        let status = if task.is_enabled() { "✅ actif" } else { "⏸  inactif" };
        let schedule_desc = task.schedule.description();
        let cmd = if task.command().len() > 35 {
            format!("{}...", &task.command()[..32])
        } else {
            task.command().to_string()
        };
        println!(
            "{:<20} {:<10} {:<35} {}",
            task.name(),
            status,
            schedule_desc,
            cmd
        );
    }
    println!();
}

fn cmd_next(registry: &TaskRegistry, name: Option<&str>) {
    let now = Local::now();

    let tasks: Vec<_> = if let Some(n) = name {
        match registry.get_task(n) {
            Ok(t) => vec![t],
            Err(e) => {
                eprintln!("❌ Erreur : {}", e);
                return;
            }
        }
    } else {
        registry.all_tasks()
    };

    println!("\n{:<20} {}", "TÂCHE", "PROCHAINE EXÉCUTION");
    println!("{}", "-".repeat(50));

    for task in tasks {
        if !task.is_enabled() {
            println!("{:<20} ⏸  Tâche désactivée", task.name());
            continue;
        }
        match task.schedule.next_occurrence(now) {
            Some(next) => {
                let diff = next.signed_duration_since(now);
                let display = if diff.num_seconds() < 60 {
                    format!("{} (dans {}s)", next.format("%Y-%m-%d %H:%M:%S"), diff.num_seconds())
                } else if diff.num_minutes() < 60 {
                    format!("{} (dans {}min)", next.format("%Y-%m-%d %H:%M:%S"), diff.num_minutes())
                } else {
                    format!("{} (dans {}h{}min)",
                        next.format("%Y-%m-%d %H:%M:%S"),
                        diff.num_hours(),
                        diff.num_minutes() % 60
                    )
                };
                println!("{:<20} {}", task.name(), display);
            }
            None => println!("{:<20} Aucune occurrence trouvée", task.name()),
        }
    }
    println!();
}

fn cmd_run(registry: &TaskRegistry, history: &HistoryManager, name: &str) {
    match registry.get_task(name) {
        Err(e) => {
            eprintln!("❌ Tâche introuvable : {}", e);
        }
        Ok(task) => {
            println!("▶ Exécution forcée de '{}' : {}", name, task.command());
            let (tx, rx) = unbounded();
            run_with_retry(task, &tx);
            drop(tx);

            for event in rx {
                if let Err(e) = history.record(&event) {
                    eprintln!("⚠ Erreur d'enregistrement: {}", e);
                }
                print_execution_result(&event);
            }
        }
    }
}

fn cmd_set_enabled(registry: &mut TaskRegistry, name: &str, enabled: bool) {
    match registry.set_enabled(name, enabled) {
        Ok(()) => {
            let action = if enabled { "activée ✅" } else { "désactivée ⏸" };
            println!("Tâche '{}' {}.", name, action);
            println!("Note: Redémarrez le daemon pour appliquer le changement.");
        }
        Err(e) => eprintln!("❌ Erreur : {}", e),
    }
}

fn cmd_health(registry: &TaskRegistry, history: &HistoryManager) {
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║          RAPPORT DE SANTÉ — TaskForge                       ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    let now = Local::now();
    println!("  Date        : {}", now.format("%Y-%m-%d %H:%M:%S"));

    let all_tasks = registry.all_tasks();
    let active_count = registry.enabled_tasks().len();
    let total_count = all_tasks.len();

    println!("  Tâches      : {} actives / {} totales", active_count, total_count);
    println!();

    if total_count == 0 {
        println!("  Aucune tâche configurée.\n");
        return;
    }

    // Stats globales
    let mut global_runs = 0u64;
    let mut global_successes = 0u64;
    let mut retarded_tasks: Vec<String> = Vec::new();
    let mut failing_tasks: Vec<String> = Vec::new();

    println!("  {:<20} {:>10} {:>10} {:>10}  {}", "TÂCHE", "EXÉC.", "SUCCÈS %", "DERNIÈRE", "STATUT");
    println!("  {}", "-".repeat(72));

    for task in &all_tasks {
        match history.stats(task.name()) {
            Err(_) => {
                println!("  {:<20} {:>10} {:>10} {:>10}  ❓ Aucune donnée", task.name(), "-", "-", "-");
            }
            Ok(stats) => {
                global_runs += stats.total_runs;
                global_successes += stats.successes;

                let last_str = match stats.last_run {
                    Some(dt) => dt.format("%d/%m %H:%M").to_string(),
                    None => "jamais".to_string(),
                };

                let status_icon = if !task.is_enabled() {
                    "⏸  Désactivé"
                } else if stats.total_runs == 0 {
                    "⚪ Jamais exécuté"
                } else if stats.success_rate >= 90.0 {
                    "✅ Sain"
                } else if stats.success_rate >= 50.0 {
                    "⚠️  Dégradé"
                } else {
                    "❌ Critique"
                };

                if stats.success_rate < 50.0 && stats.total_runs > 0 {
                    failing_tasks.push(task.name().to_string());
                }

                // Tâches en retard : dernière exécution > 2x la période prévue
                // (simplification : on vérifie si la prochaine était il y a plus de 5min)
                if task.is_enabled() {
                    if let Some(next) = task.schedule.next_occurrence(now - chrono::Duration::hours(1)) {
                        if next < now - chrono::Duration::minutes(5) {
                            retarded_tasks.push(task.name().to_string());
                        }
                    }
                }

                println!(
                    "  {:<20} {:>10} {:>9.1}% {:>10}  {}",
                    task.name(),
                    stats.total_runs,
                    stats.success_rate,
                    last_str,
                    status_icon
                );
            }
        }
    }

    println!();

    // Résumé global
    let global_rate = if global_runs > 0 {
        global_successes as f64 / global_runs as f64 * 100.0
    } else {
        0.0
    };

    println!("  ──────────────────────────────────────────────────────────────");
    println!("  TOTAL          {:>10}    {:.1}% de succès global", global_runs, global_rate);

    if !failing_tasks.is_empty() {
        println!("\n  ⚠️  Tâches en échec (< 50%) : {}", failing_tasks.join(", "));
    }
    if !retarded_tasks.is_empty() {
        println!("  🕐 Tâches en retard         : {}", retarded_tasks.join(", "));
    }
    if failing_tasks.is_empty() && retarded_tasks.is_empty() {
        println!("\n  ✅ Tous les systèmes sont opérationnels.");
    }

    println!();
}

fn cmd_daemon(registry: &TaskRegistry, history: &HistoryManager, history_dir: &PathBuf) {
    use std::sync::Arc;
    use std::sync::Mutex;
    use crate::executor::Supervisor;

    println!("🚀 Démarrage du daemon TaskForge...");
    println!("   Appuyez sur Ctrl+C pour arrêter.\n");

    let (event_tx, event_rx) = unbounded();

    // Lancer le gestionnaire d'historique dans un thread
    let history_manager = HistoryManager::new(history_dir);
    let history_handle = history_manager.start_listener(event_rx);

    // Construire un registre à partir des tâches actives
    let mut new_registry = TaskRegistry::new();
    for task in registry.enabled_tasks() {
        // On recrée les tâches pour le registre du daemon
        if let Err(e) = new_registry.add_task(task.config.clone()) {
            eprintln!("⚠ Impossible d'ajouter la tâche '{}': {}", task.name(), e);
        }
    }

    let shared_registry = Arc::new(Mutex::new(new_registry));
    let mut supervisor = Supervisor::new(shared_registry, event_tx);

    // Gérer Ctrl+C
    // (Dans un vrai daemon, on utiliserait ctrlc ou signal-hook)
    println!("Daemon actif. Tâches surveillées :");
    for task in registry.enabled_tasks() {
        let now = Local::now();
        if let Some(next) = task.schedule.next_occurrence(now) {
            println!("  - {} → prochaine exécution : {}", task.name(), next.format("%H:%M:%S"));
        }
    }
    println!();

    supervisor.run(); // Bloquant
    history_handle.join().ok();
}

/// Affiche le résultat d'une exécution de manière formatée
fn print_execution_result(event: &ExecutionEvent) {
    let icon = if event.success { "✅" } else if event.timed_out { "⏱" } else { "❌" };
    println!("\n{} Résultat de '{}'", icon, event.task_name);
    println!("   Démarré   : {}", event.started_at.format("%Y-%m-%d %H:%M:%S"));
    println!("   Terminé   : {}", event.finished_at.format("%Y-%m-%d %H:%M:%S"));
    println!("   Durée     : {:.3}s", event.duration_secs);
    println!("   Code      : {:?}", event.exit_code);
    if event.timed_out {
        println!("   ⚠ TIMEOUT atteint !");
    }
    if !event.stdout.trim().is_empty() {
        println!("   Stdout    :\n{}", indent(&event.stdout, "     "));
    }
    if !event.stderr.trim().is_empty() {
        println!("   Stderr    :\n{}", indent(&event.stderr, "     "));
    }
}

fn indent(s: &str, prefix: &str) -> String {
    s.lines().map(|l| format!("{}{}", prefix, l)).collect::<Vec<_>>().join("\n")
}

// ============================================================
// TESTS — Partie 5 (tests d'intégration CLI)
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::registry::{TaskConfig, RetryPolicy};

    fn make_registry() -> TaskRegistry {
        let toml = r#"
[[tasks]]
name = "tache_active"
command = "echo hello"
schedule = "@hourly"
enabled = true

[[tasks]]
name = "tache_inactive"
command = "echo world"
schedule = "@daily"
enabled = false
"#;
        TaskRegistry::load_from_str(toml).unwrap()
    }

    #[test]
    fn test_list_toutes() {
        let registry = make_registry();
        // Vérifie que les deux tâches existent
        assert_eq!(registry.all_tasks().len(), 2);
    }

    #[test]
    fn test_list_actives_seulement() {
        let registry = make_registry();
        assert_eq!(registry.enabled_tasks().len(), 1);
        assert_eq!(registry.enabled_tasks()[0].name(), "tache_active");
    }

    #[test]
    fn test_next_occurrence_cli() {
        let registry = make_registry();
        let now = Local::now();
        let task = registry.get_task("tache_active").unwrap();
        let next = task.schedule.next_occurrence(now);
        assert!(next.is_some());
        assert!(next.unwrap() > now);
    }

    #[test]
    fn test_run_force_enregistre_historique() {
        let dir = TempDir::new().unwrap();
        let history = HistoryManager::new(dir.path());
        let registry = make_registry();

        let (tx, rx) = unbounded();
        let task = registry.get_task("tache_active").unwrap();
        run_with_retry(task, &tx);
        drop(tx);

        for event in rx {
            history.record(&event).unwrap();
        }

        let events = history.read_all("tache_active").unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].success);
    }

    #[test]
    fn test_enable_disable() {
        let mut registry = make_registry();
        // Désactiver une tâche active
        registry.set_enabled("tache_active", false).unwrap();
        assert_eq!(registry.enabled_tasks().len(), 0);
        // Réactiver
        registry.set_enabled("tache_active", true).unwrap();
        assert_eq!(registry.enabled_tasks().len(), 1);
    }

    #[test]
    fn test_health_sans_historique() {
        let dir = TempDir::new().unwrap();
        let history = HistoryManager::new(dir.path());
        let registry = make_registry();
        // Pas de panique même sans données d'historique
        cmd_health(&registry, &history);
    }

    #[test]
    fn test_health_avec_historique() {
        let dir = TempDir::new().unwrap();
        let history = HistoryManager::new(dir.path());
        let registry = make_registry();

        // Ajouter quelques exécutions
        let (tx, rx) = unbounded();
        let task = registry.get_task("tache_active").unwrap();
        run_with_retry(task, &tx);
        drop(tx);
        for event in rx {
            history.record(&event).unwrap();
        }

        cmd_health(&registry, &history);
    }
}
