// ============================================================
// TaskForge — Planificateur de tâches système (cron-like)
// Projet 5 — Cours Rust ENSPD
// ============================================================

mod scheduler;
mod registry;
mod executor;
mod history;
mod cli;

use clap::Parser;
use cli::{Cli, run_command};
use history::HistoryManager;
use registry::TaskRegistry;
use log::{error, info};

fn main() {
    // Initialiser le logger (RUST_LOG=info pour voir les logs)
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    ).init();

    let cli = Cli::parse();

    // Charger la configuration
    let mut registry = match TaskRegistry::load_from_file(&cli.config) {
        Ok(r) => {
            info!("Configuration chargée : {} tâche(s)", r.len());
            r
        }
        Err(e) => {
            error!("Impossible de charger la configuration '{}': {}", cli.config.display(), e);
            eprintln!("❌ Erreur de configuration : {}", e);
            eprintln!("   Vérifiez que le fichier '{}' existe et est valide.", cli.config.display());
            std::process::exit(1);
        }
    };

    // Initialiser le gestionnaire d'historique
    let history = HistoryManager::new(&cli.history_dir);

    // Exécuter la commande
    run_command(&cli, &mut registry, &history);
}
