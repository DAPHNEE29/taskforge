# TaskForge — Planificateur de tâches système (cron-like)
## Projet 5 — Cours Rust ENSPD

---

## Structure du projet

```
taskforge/
├── Cargo.toml
├── README.md
├── config/
│   └── tasks.toml          ← configuration des tâches
├── history/                ← créé automatiquement
│   └── <nom_tache>/
│       └── YYYY-MM.jsonl
└── src/
    ├── main.rs
    ├── scheduler/mod.rs    ← PARTIE 1 : Parser cron + next_occurrence
    ├── registry/mod.rs     ← PARTIE 2 : Task, TaskConfig, TaskRegistry
    ├── registry/task_ext.rs
    ├── executor/mod.rs     ← PARTIE 3 : Threads, timeout, retry
    ├── history/mod.rs      ← PARTIE 4 : JSONL, stats, listener
    └── cli/mod.rs          ← PARTIE 5 : CLI, rapport de santé
```

---

## Installation de Rust (si pas encore fait)

```bash
# Linux / macOS
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Windows : télécharger rustup-init.exe sur https://rustup.rs
```

Vérifier :
```bash
rustc --version
cargo --version
```

---

## Compilation

```bash
cd taskforge
cargo build
```

---

## Lancer tous les tests

```bash
# Tous les tests d'un coup
cargo test

# Par partie
cargo test scheduler    # Partie 1
cargo test registry     # Partie 2
cargo test executor     # Partie 3
cargo test history      # Partie 4
cargo test cli          # Partie 5

# Avec affichage des println!
cargo test -- --nocapture
```

---

## Commandes CLI

```bash
# Lister toutes les tâches
cargo run -- list

# Lister seulement les tâches actives
cargo run -- list --enabled-only

# Voir les prochaines exécutions
cargo run -- next
cargo run -- next test_rapide

# Forcer une exécution immédiate
cargo run -- run test_rapide
cargo run -- run backup_db

# Activer / désactiver une tâche
cargo run -- enable rapport_hebdo
cargo run -- disable nettoyage_tmp

# Rapport de santé complet
cargo run -- health

# Lancer le daemon (planificateur)
cargo run -- daemon
# Ctrl+C pour arrêter
```

---

## Format config/tasks.toml

```toml
[[tasks]]
name        = "ma_tache"
command     = "echo hello"
schedule    = "@every 5m"
timeout_secs = 30
description = "Description"
enabled     = true

[tasks.retry]
kind        = "fixed"       # none | immediate | fixed | exponential
max_retries = 3
delay_secs  = 10
```

### Expressions de planification

| Expression      | Signification               |
|-----------------|-----------------------------|
| `* * * * *`     | Chaque minute               |
| `0 * * * *`     | Chaque heure                |
| `30 8 * * *`    | Chaque jour à 8h30          |
| `*/15 * * * *`  | Toutes les 15 minutes       |
| `@daily`        | Chaque jour à minuit        |
| `@hourly`       | Chaque heure                |
| `@weekly`       | Chaque dimanche à minuit    |
| `@monthly`      | Le 1er du mois à minuit     |
| `@every 5m`     | Toutes les 5 minutes        |
| `@every 2h`     | Toutes les 2 heures         |
| `@every 30s`    | Toutes les 30 secondes      |

---

## Résumé des parties

| Partie | Fichier                  | Tests |
|--------|--------------------------|-------|
| 1      | `src/scheduler/mod.rs`   | 14    |
| 2      | `src/registry/mod.rs`    | 10    |
| 3      | `src/executor/mod.rs`    | 7     |
| 4      | `src/history/mod.rs`     | 9     |
| 5      | `src/cli/mod.rs`         | 6     |
