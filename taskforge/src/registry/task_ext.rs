// Extension de Task pour créer depuis config + schedule déjà parsé
// Nécessaire pour passer les données dans les threads du Supervisor

use crate::registry::{TaskConfig, Task};
use crate::scheduler::Schedule;
use crate::registry::RegistryError;

impl Task {
    /// Crée une Task depuis une config et un Schedule déjà parsé (pour les threads)
    pub fn from_config_with_schedule(config: TaskConfig, schedule: Schedule) -> Self {
        Task { config, schedule }
    }
}
