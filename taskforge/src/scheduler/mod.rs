// ============================================================
// PARTIE 1 — Parser d'expressions de planification (cron-like)
// ============================================================
// Supporte :
//   - Expressions 5 champs : "minute heure jour mois jour_semaine"
//   - Macros : @daily, @hourly, @weekly, @monthly, @every Xm/Xh/Xs
//   - Wildcards (*), listes (1,2,3), intervalles (1-5), pas (*/2)
// ============================================================

use chrono::{DateTime, Datelike, Local, Timelike};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScheduleError {
    #[error("Expression invalide: {0}")]
    ParseError(String),
    #[error("Valeur hors limites: champ={champ}, valeur={valeur}")]
    OutOfRange { champ: String, valeur: u32 },
}

/// Représente un champ cron : peut être "tous", une liste de valeurs ou un intervalle avec pas
#[derive(Debug, Clone, PartialEq)]
pub enum CronField {
    All,                      // *
    Values(Vec<u32>),         // 1,2,5 ou 3-7 ou */2 => liste résolue
}

impl CronField {
    /// Parse un champ cron (ex: "*/5", "1,2,3", "0-23", "*")
    pub fn parse(s: &str, min: u32, max: u32) -> Result<Self, ScheduleError> {
        if s == "*" {
            return Ok(CronField::All);
        }

        let mut values = Vec::new();

        for part in s.split(',') {
            if part.contains('/') {
                // Pas : "*/5" ou "0-59/5"
                let parts: Vec<&str> = part.splitn(2, '/').collect();
                let step: u32 = parts[1].parse().map_err(|_| {
                    ScheduleError::ParseError(format!("Pas invalide: {}", parts[1]))
                })?;
                if step == 0 {
                    return Err(ScheduleError::ParseError("Le pas ne peut pas être 0".into()));
                }
                let (range_min, range_max) = if parts[0] == "*" {
                    (min, max)
                } else if parts[0].contains('-') {
                    let bounds: Vec<&str> = parts[0].splitn(2, '-').collect();
                    let a: u32 = bounds[0].parse().map_err(|_| {
                        ScheduleError::ParseError(format!("Borne invalide: {}", bounds[0]))
                    })?;
                    let b: u32 = bounds[1].parse().map_err(|_| {
                        ScheduleError::ParseError(format!("Borne invalide: {}", bounds[1]))
                    })?;
                    (a, b)
                } else {
                    let v: u32 = parts[0].parse().map_err(|_| {
                        ScheduleError::ParseError(format!("Valeur invalide: {}", parts[0]))
                    })?;
                    (v, max)
                };
                let mut v = range_min;
                while v <= range_max {
                    Self::check_range(&v.to_string(), v, min, max)?;
                    values.push(v);
                    v += step;
                }
            } else if part.contains('-') {
                // Intervalle : "1-5"
                let bounds: Vec<&str> = part.splitn(2, '-').collect();
                let a: u32 = bounds[0].parse().map_err(|_| {
                    ScheduleError::ParseError(format!("Borne invalide: {}", bounds[0]))
                })?;
                let b: u32 = bounds[1].parse().map_err(|_| {
                    ScheduleError::ParseError(format!("Borne invalide: {}", bounds[1]))
                })?;
                if a > b {
                    return Err(ScheduleError::ParseError(format!(
                        "Intervalle invalide: {}-{}", a, b
                    )));
                }
                for v in a..=b {
                    Self::check_range(&v.to_string(), v, min, max)?;
                    values.push(v);
                }
            } else {
                // Valeur simple
                let v: u32 = part.parse().map_err(|_| {
                    ScheduleError::ParseError(format!("Valeur invalide: {}", part))
                })?;
                Self::check_range(part, v, min, max)?;
                values.push(v);
            }
        }

        values.sort_unstable();
        values.dedup();
        Ok(CronField::Values(values))
    }

    fn check_range(s: &str, v: u32, min: u32, max: u32) -> Result<(), ScheduleError> {
        if v < min || v > max {
            return Err(ScheduleError::OutOfRange {
                champ: s.to_string(),
                valeur: v,
            });
        }
        Ok(())
    }

    /// Vérifie si une valeur correspond à ce champ
    pub fn matches(&self, value: u32) -> bool {
        match self {
            CronField::All => true,
            CronField::Values(vals) => vals.contains(&value),
        }
    }
}

/// Représente une planification complète
#[derive(Debug, Clone)]
pub enum Schedule {
    /// Expression cron 5 champs
    Cron {
        minute: CronField,
        hour: CronField,
        day_of_month: CronField,
        month: CronField,
        day_of_week: CronField,
    },
    /// Intervalle fixe en secondes (pour @every)
    Interval(std::time::Duration),
}

impl Schedule {
    /// Parse une expression de planification
    /// Supporte : "* * * * *", @daily, @hourly, @weekly, @monthly, @every Xm, @every Xh, @every Xs
    pub fn parse(expr: &str) -> Result<Self, ScheduleError> {
        let expr = expr.trim();

        // Macros spéciales
        match expr {
            "@daily" | "@midnight" => {
                return Schedule::parse("0 0 * * *");
            }
            "@hourly" => {
                return Schedule::parse("0 * * * *");
            }
            "@weekly" => {
                return Schedule::parse("0 0 * * 0");
            }
            "@monthly" => {
                return Schedule::parse("0 0 1 * *");
            }
            "@yearly" | "@annually" => {
                return Schedule::parse("0 0 1 1 *");
            }
            _ => {}
        }

        // @every Xm / Xh / Xs
        if let Some(rest) = expr.strip_prefix("@every ") {
            let rest = rest.trim();
            if let Some(mins) = rest.strip_suffix('m') {
                let n: u64 = mins.parse().map_err(|_| {
                    ScheduleError::ParseError(format!("@every invalide: {}", expr))
                })?;
                return Ok(Schedule::Interval(std::time::Duration::from_secs(n * 60)));
            } else if let Some(hours) = rest.strip_suffix('h') {
                let n: u64 = hours.parse().map_err(|_| {
                    ScheduleError::ParseError(format!("@every invalide: {}", expr))
                })?;
                return Ok(Schedule::Interval(std::time::Duration::from_secs(n * 3600)));
            } else if let Some(secs) = rest.strip_suffix('s') {
                let n: u64 = secs.parse().map_err(|_| {
                    ScheduleError::ParseError(format!("@every invalide: {}", expr))
                })?;
                return Ok(Schedule::Interval(std::time::Duration::from_secs(n)));
            } else {
                return Err(ScheduleError::ParseError(format!(
                    "@every doit être suivi de Xm, Xh ou Xs, reçu: {}",
                    rest
                )));
            }
        }

        // Expression 5 champs
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(ScheduleError::ParseError(format!(
                "L'expression cron doit avoir 5 champs, reçu {} : '{}'",
                fields.len(),
                expr
            )));
        }

        Ok(Schedule::Cron {
            minute:       CronField::parse(fields[0], 0, 59)?,
            hour:         CronField::parse(fields[1], 0, 23)?,
            day_of_month: CronField::parse(fields[2], 1, 31)?,
            month:        CronField::parse(fields[3], 1, 12)?,
            day_of_week:  CronField::parse(fields[4], 0, 6)?,
        })
    }

    /// Calcule la prochaine occurrence à partir d'un DateTime donné
    pub fn next_occurrence(&self, from: DateTime<Local>) -> Option<DateTime<Local>> {
        match self {
            Schedule::Interval(duration) => {
                let secs = duration.as_secs() as i64;
                Some(from + chrono::Duration::seconds(secs))
            }
            Schedule::Cron {
                minute,
                hour,
                day_of_month,
                month,
                day_of_week,
            } => {
                // On commence à la minute suivante
                let mut t = from + chrono::Duration::minutes(1);
                // On met les secondes à 0
                t = t.with_second(0).unwrap().with_nanosecond(0).unwrap();

                // Recherche sur au max 4 ans (évite boucle infinie)
                let limit = from + chrono::Duration::days(4 * 366);

                while t < limit {
                    // Vérifier le mois
                    if !month.matches(t.month()) {
                        // Avancer au 1er du mois suivant
                        t = advance_to_next_month(t)?;
                        continue;
                    }
                    // Vérifier le jour du mois
                    if !day_of_month.matches(t.day()) {
                        t = advance_to_next_day(t)?;
                        continue;
                    }
                    // Vérifier le jour de la semaine (0=Dimanche)
                    let dow = t.weekday().num_days_from_sunday();
                    if !day_of_week.matches(dow) {
                        t = advance_to_next_day(t)?;
                        continue;
                    }
                    // Vérifier l'heure
                    if !hour.matches(t.hour()) {
                        t = advance_to_next_hour(t)?;
                        continue;
                    }
                    // Vérifier la minute
                    if !minute.matches(t.minute()) {
                        t = t + chrono::Duration::minutes(1);
                        continue;
                    }
                    return Some(t);
                }
                None
            }
        }
    }

    /// Affichage lisible de la planification
    pub fn description(&self) -> String {
        match self {
            Schedule::Interval(d) => {
                let secs = d.as_secs();
                if secs % 3600 == 0 {
                    format!("Toutes les {} heure(s)", secs / 3600)
                } else if secs % 60 == 0 {
                    format!("Toutes les {} minute(s)", secs / 60)
                } else {
                    format!("Toutes les {} seconde(s)", secs)
                }
            }
            Schedule::Cron { .. } => "Expression cron".into(),
        }
    }
}

// ---- Helpers pour avancer dans le temps ----

fn advance_to_next_month(t: DateTime<Local>) -> Option<DateTime<Local>> {
    let (year, month) = if t.month() == 12 {
        (t.year() + 1, 1u32)
    } else {
        (t.year(), t.month() + 1)
    };
    t.with_year(year)?
        .with_month(month)?
        .with_day(1)?
        .with_hour(0)?
        .with_minute(0)?
        .with_second(0)?
        .with_nanosecond(0)
}

fn advance_to_next_day(t: DateTime<Local>) -> Option<DateTime<Local>> {
    let next = t + chrono::Duration::days(1);
    next.with_hour(0)?
        .with_minute(0)?
        .with_second(0)?
        .with_nanosecond(0)
}

fn advance_to_next_hour(t: DateTime<Local>) -> Option<DateTime<Local>> {
    let next = t + chrono::Duration::hours(1);
    next.with_minute(0)?
        .with_second(0)?
        .with_nanosecond(0)
}

// ============================================================
// TESTS — Partie 1
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dt(y: i32, mo: u32, d: u32, h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, m, 0).unwrap()
    }

    // --- Tests du parser ---

    #[test]
    fn test_wildcard() {
        let f = CronField::parse("*", 0, 59).unwrap();
        assert_eq!(f, CronField::All);
        assert!(f.matches(0));
        assert!(f.matches(59));
    }

    #[test]
    fn test_valeur_simple() {
        let f = CronField::parse("5", 0, 59).unwrap();
        assert!(f.matches(5));
        assert!(!f.matches(6));
    }

    #[test]
    fn test_liste() {
        let f = CronField::parse("1,15,30", 0, 59).unwrap();
        assert!(f.matches(1));
        assert!(f.matches(15));
        assert!(f.matches(30));
        assert!(!f.matches(2));
    }

    #[test]
    fn test_intervalle() {
        let f = CronField::parse("0-4", 0, 59).unwrap();
        for i in 0..=4 {
            assert!(f.matches(i));
        }
        assert!(!f.matches(5));
    }

    #[test]
    fn test_pas() {
        let f = CronField::parse("*/15", 0, 59).unwrap();
        assert!(f.matches(0));
        assert!(f.matches(15));
        assert!(f.matches(30));
        assert!(f.matches(45));
        assert!(!f.matches(1));
    }

    #[test]
    fn test_erreur_hors_limites() {
        let result = CronField::parse("60", 0, 59);
        assert!(result.is_err());
    }

    #[test]
    fn test_erreur_pas_zero() {
        let result = CronField::parse("*/0", 0, 59);
        assert!(result.is_err());
    }

    // --- Tests des macros ---

    #[test]
    fn test_macro_daily() {
        let s = Schedule::parse("@daily").unwrap();
        let from = dt(2024, 1, 1, 10, 30);
        let next = s.next_occurrence(from).unwrap();
        assert_eq!(next.hour(), 0);
        assert_eq!(next.minute(), 0);
        assert_eq!(next.day(), 2);
    }

    #[test]
    fn test_macro_hourly() {
        let s = Schedule::parse("@hourly").unwrap();
        let from = dt(2024, 1, 1, 10, 30);
        let next = s.next_occurrence(from).unwrap();
        assert_eq!(next.hour(), 11);
        assert_eq!(next.minute(), 0);
    }

    #[test]
    fn test_macro_every_5m() {
        let s = Schedule::parse("@every 5m").unwrap();
        let from = dt(2024, 1, 1, 10, 0);
        let next = s.next_occurrence(from).unwrap();
        assert_eq!(next, from + chrono::Duration::minutes(5));
    }

    #[test]
    fn test_macro_every_2h() {
        let s = Schedule::parse("@every 2h").unwrap();
        let from = dt(2024, 1, 1, 10, 0);
        let next = s.next_occurrence(from).unwrap();
        assert_eq!(next, from + chrono::Duration::hours(2));
    }

    #[test]
    fn test_macro_every_30s() {
        let s = Schedule::parse("@every 30s").unwrap();
        let from = dt(2024, 1, 1, 10, 0);
        let next = s.next_occurrence(from).unwrap();
        assert_eq!(next, from + chrono::Duration::seconds(30));
    }

    // --- Tests next_occurrence ---

    #[test]
    fn test_next_occurrence_chaque_minute() {
        // "* * * * *" = chaque minute
        let s = Schedule::parse("* * * * *").unwrap();
        let from = dt(2024, 1, 15, 10, 30);
        let next = s.next_occurrence(from).unwrap();
        assert_eq!(next, dt(2024, 1, 15, 10, 31));
    }

    #[test]
    fn test_next_occurrence_specifique() {
        // "30 14 * * *" = chaque jour à 14h30
        let s = Schedule::parse("30 14 * * *").unwrap();
        let from = dt(2024, 1, 15, 10, 0);
        let next = s.next_occurrence(from).unwrap();
        assert_eq!(next.hour(), 14);
        assert_eq!(next.minute(), 30);
    }

    #[test]
    fn test_next_occurrence_franchit_minuit() {
        // "0 1 * * *" = chaque jour à 1h00
        let s = Schedule::parse("0 1 * * *").unwrap();
        let from = dt(2024, 1, 15, 10, 0);
        let next = s.next_occurrence(from).unwrap();
        assert_eq!(next.day(), 16);
        assert_eq!(next.hour(), 1);
        assert_eq!(next.minute(), 0);
    }

    #[test]
    fn test_next_occurrence_franchit_mois() {
        // "0 0 1 * *" = le 1er de chaque mois à minuit
        let s = Schedule::parse("0 0 1 * *").unwrap();
        let from = dt(2024, 1, 15, 10, 0);
        let next = s.next_occurrence(from).unwrap();
        assert_eq!(next.month(), 2);
        assert_eq!(next.day(), 1);
        assert_eq!(next.hour(), 0);
    }

    #[test]
    fn test_erreur_expression_incomplete() {
        let result = Schedule::parse("* * *");
        assert!(result.is_err());
    }

    #[test]
    fn test_erreur_macro_every_invalide() {
        let result = Schedule::parse("@every 5x");
        assert!(result.is_err());
    }
}
