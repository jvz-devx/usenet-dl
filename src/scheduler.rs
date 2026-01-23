//! Time-based scheduler for applying speed limits and pause/resume based on schedules.
//!
//! The scheduler allows users to define rules that automatically adjust download behavior
//! based on the time of day and day of week. Common use cases include:
//! - Limiting speed during work hours to preserve bandwidth
//! - Running unlimited during off-peak hours (nights/weekends)
//! - Pausing downloads during specific time windows
//!
//! # Example
//!
//! ```rust
//! use usenet_dl::scheduler::{ScheduleRule, ScheduleAction, Weekday};
//! use chrono::NaiveTime;
//!
//! // Unlimited at night (midnight to 6 AM)
//! let night_rule = ScheduleRule {
//!     id: 1,
//!     name: "Night owl".into(),
//!     days: vec![],  // All days
//!     start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
//!     end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
//!     action: ScheduleAction::Unlimited,
//!     enabled: true,
//! };
//!
//! // Limited during work hours (weekdays only)
//! let work_rule = ScheduleRule {
//!     id: 2,
//!     name: "Work hours".into(),
//!     days: vec![Weekday::Monday, Weekday::Tuesday, Weekday::Wednesday,
//!                Weekday::Thursday, Weekday::Friday],
//!     start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
//!     end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
//!     action: ScheduleAction::SpeedLimit(1_000_000),  // 1 MB/s
//!     enabled: true,
//! };
//! ```

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};

/// Unique identifier for a schedule rule
pub type RuleId = i64;

/// A time-based schedule rule that applies an action during specific time windows
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ScheduleRule {
    /// Unique identifier for this rule
    pub id: RuleId,

    /// Human-readable name for this rule
    pub name: String,

    /// Days this rule applies (empty = all days)
    pub days: Vec<Weekday>,

    /// Start time (HH:MM:SS, 24-hour format)
    #[serde(with = "time_format")]
    pub start_time: NaiveTime,

    /// End time (HH:MM:SS, 24-hour format)
    #[serde(with = "time_format")]
    pub end_time: NaiveTime,

    /// Action to take during this time window
    pub action: ScheduleAction,

    /// Whether this rule is currently active
    pub enabled: bool,
}

/// Action to take when a schedule rule is active
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value")]
pub enum ScheduleAction {
    /// Set speed limit in bytes per second
    SpeedLimit(u64),
    /// Remove speed limit (unlimited speed)
    Unlimited,
    /// Pause all downloads
    Pause,
}

/// Days of the week for schedule rules
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Weekday {
    /// Monday
    Monday,
    /// Tuesday
    Tuesday,
    /// Wednesday
    Wednesday,
    /// Thursday
    Thursday,
    /// Friday
    Friday,
    /// Saturday
    Saturday,
    /// Sunday
    Sunday,
}

impl Weekday {
    /// Convert from chrono::Weekday to our Weekday
    pub fn from_chrono(wd: chrono::Weekday) -> Self {
        use chrono::Weekday as ChronoWd;
        match wd {
            ChronoWd::Mon => Weekday::Monday,
            ChronoWd::Tue => Weekday::Tuesday,
            ChronoWd::Wed => Weekday::Wednesday,
            ChronoWd::Thu => Weekday::Thursday,
            ChronoWd::Fri => Weekday::Friday,
            ChronoWd::Sat => Weekday::Saturday,
            ChronoWd::Sun => Weekday::Sunday,
        }
    }

    /// Convert to chrono::Weekday
    pub fn to_chrono(self) -> chrono::Weekday {
        use chrono::Weekday as ChronoWd;
        match self {
            Weekday::Monday => ChronoWd::Mon,
            Weekday::Tuesday => ChronoWd::Tue,
            Weekday::Wednesday => ChronoWd::Wed,
            Weekday::Thursday => ChronoWd::Thu,
            Weekday::Friday => ChronoWd::Fri,
            Weekday::Saturday => ChronoWd::Sat,
            Weekday::Sunday => ChronoWd::Sun,
        }
    }
}

/// Serde module for serializing/deserializing NaiveTime as HH:MM:SS strings
mod time_format {
    use chrono::NaiveTime;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(time: &NaiveTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = time.format("%H:%M:%S").to_string();
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<NaiveTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        NaiveTime::parse_from_str(&s, "%H:%M:%S").map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_rule_creation() {
        let rule = ScheduleRule {
            id: 1,
            name: "Test Rule".into(),
            days: vec![Weekday::Monday, Weekday::Friday],
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            action: ScheduleAction::SpeedLimit(1_000_000),
            enabled: true,
        };

        assert_eq!(rule.id, 1);
        assert_eq!(rule.name, "Test Rule");
        assert_eq!(rule.days.len(), 2);
        assert!(rule.enabled);
    }

    #[test]
    fn test_schedule_action_variants() {
        let limit = ScheduleAction::SpeedLimit(5_000_000);
        let unlimited = ScheduleAction::Unlimited;
        let pause = ScheduleAction::Pause;

        assert!(matches!(limit, ScheduleAction::SpeedLimit(5_000_000)));
        assert!(matches!(unlimited, ScheduleAction::Unlimited));
        assert!(matches!(pause, ScheduleAction::Pause));
    }

    #[test]
    fn test_weekday_conversion() {
        use chrono::Weekday as ChronoWd;

        assert_eq!(
            Weekday::from_chrono(ChronoWd::Mon),
            Weekday::Monday
        );
        assert_eq!(
            Weekday::from_chrono(ChronoWd::Fri),
            Weekday::Friday
        );
        assert_eq!(
            Weekday::from_chrono(ChronoWd::Sun),
            Weekday::Sunday
        );

        assert_eq!(Weekday::Monday.to_chrono(), ChronoWd::Mon);
        assert_eq!(Weekday::Friday.to_chrono(), ChronoWd::Fri);
        assert_eq!(Weekday::Sunday.to_chrono(), ChronoWd::Sun);
    }

    #[test]
    fn test_weekday_round_trip() {
        use chrono::Weekday as ChronoWd;

        let days = vec![
            ChronoWd::Mon,
            ChronoWd::Tue,
            ChronoWd::Wed,
            ChronoWd::Thu,
            ChronoWd::Fri,
            ChronoWd::Sat,
            ChronoWd::Sun,
        ];

        for day in days {
            let our_day = Weekday::from_chrono(day);
            let back_to_chrono = our_day.to_chrono();
            assert_eq!(day, back_to_chrono);
        }
    }

    #[test]
    fn test_schedule_rule_serialization() {
        let rule = ScheduleRule {
            id: 42,
            name: "Work hours".into(),
            days: vec![Weekday::Monday, Weekday::Tuesday],
            start_time: NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            action: ScheduleAction::SpeedLimit(2_000_000),
            enabled: true,
        };

        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: ScheduleRule = serde_json::from_str(&json).unwrap();

        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_time_format_serialization() {
        let time = NaiveTime::from_hms_opt(14, 30, 45).unwrap();

        #[derive(Serialize, Deserialize)]
        struct TestStruct {
            #[serde(with = "time_format")]
            time: NaiveTime,
        }

        let test = TestStruct { time };
        let json = serde_json::to_string(&test).unwrap();
        assert!(json.contains("14:30:45"));

        let deserialized: TestStruct = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.time, time);
    }

    #[test]
    fn test_schedule_action_serialization() {
        let actions = vec![
            ScheduleAction::SpeedLimit(1_000_000),
            ScheduleAction::Unlimited,
            ScheduleAction::Pause,
        ];

        for action in actions {
            let json = serde_json::to_string(&action).unwrap();
            let deserialized: ScheduleAction = serde_json::from_str(&json).unwrap();
            assert_eq!(action, deserialized);
        }
    }

    #[test]
    fn test_empty_days_means_all_days() {
        let rule = ScheduleRule {
            id: 1,
            name: "Every day".into(),
            days: vec![],  // Empty = all days
            start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
            action: ScheduleAction::Unlimited,
            enabled: true,
        };

        assert!(rule.days.is_empty());
    }

    #[test]
    fn test_schedule_rule_with_all_weekdays() {
        let rule = ScheduleRule {
            id: 2,
            name: "Weekdays only".into(),
            days: vec![
                Weekday::Monday,
                Weekday::Tuesday,
                Weekday::Wednesday,
                Weekday::Thursday,
                Weekday::Friday,
            ],
            start_time: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
            action: ScheduleAction::SpeedLimit(5_000_000),
            enabled: true,
        };

        assert_eq!(rule.days.len(), 5);
        assert!(rule.days.contains(&Weekday::Monday));
        assert!(rule.days.contains(&Weekday::Friday));
        assert!(!rule.days.contains(&Weekday::Saturday));
    }
}
