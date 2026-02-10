use super::*;
use chrono::Timelike;

#[test]
fn test_schedule_rule_creation() {
    let rule = ScheduleRule {
        id: RuleId(1),
        name: "Test Rule".into(),
        days: vec![Weekday::Monday, Weekday::Friday],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::SpeedLimit(1_000_000),
        enabled: true,
    };

    assert_eq!(rule.id, RuleId(1));
    assert_eq!(rule.name, "Test Rule");
    assert_eq!(rule.days.len(), 2);
    assert!(rule.enabled);
}

#[test]
fn test_weekday_conversion() {
    use chrono::Weekday as ChronoWd;

    assert_eq!(Weekday::from_chrono(ChronoWd::Mon), Weekday::Monday);
    assert_eq!(Weekday::from_chrono(ChronoWd::Fri), Weekday::Friday);
    assert_eq!(Weekday::from_chrono(ChronoWd::Sun), Weekday::Sunday);

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
        id: RuleId(42),
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
fn test_schedule_rule_with_all_weekdays() {
    let rule = ScheduleRule {
        id: RuleId(2),
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

#[test]
fn test_scheduler_creation() {
    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Test rule".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    }];

    let scheduler = Scheduler::new(rules.clone());
    assert_eq!(scheduler.rules().len(), 1);
    assert_eq!(scheduler.rules()[0].name, "Test rule");
}

#[test]
fn test_scheduler_default() {
    let scheduler = Scheduler::default();
    assert_eq!(scheduler.rules().len(), 0);
}

#[test]
fn test_scheduler_add_rule() {
    let mut scheduler = Scheduler::default();
    assert_eq!(scheduler.rules().len(), 0);

    let rule = ScheduleRule {
        id: RuleId(1),
        name: "New rule".into(),
        days: vec![Weekday::Monday],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::SpeedLimit(1_000_000),
        enabled: true,
    };

    scheduler.add_rule(rule.clone());
    assert_eq!(scheduler.rules().len(), 1);
    assert_eq!(scheduler.rules()[0].id, RuleId(1));
    assert_eq!(scheduler.rules()[0].name, "New rule");
}

#[test]
fn test_scheduler_remove_rule() {
    let rules = vec![
        ScheduleRule {
            id: RuleId(1),
            name: "Rule 1".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
            action: ScheduleAction::Unlimited,
            enabled: true,
        },
        ScheduleRule {
            id: RuleId(2),
            name: "Rule 2".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            action: ScheduleAction::Pause,
            enabled: true,
        },
    ];

    let mut scheduler = Scheduler::new(rules);
    assert_eq!(scheduler.rules().len(), 2);

    // Remove existing rule
    let removed = scheduler.remove_rule(RuleId(1));
    assert!(removed);
    assert_eq!(scheduler.rules().len(), 1);
    assert_eq!(scheduler.rules()[0].id, RuleId(2));

    // Try to remove non-existent rule
    let not_removed = scheduler.remove_rule(RuleId(99));
    assert!(!not_removed);
    assert_eq!(scheduler.rules().len(), 1);
}

#[test]
fn test_scheduler_update_rule() {
    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Original rule".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    }];

    let mut scheduler = Scheduler::new(rules);
    assert_eq!(scheduler.rules()[0].name, "Original rule");

    // Update existing rule
    let updated_rule = ScheduleRule {
        id: RuleId(1),
        name: "Updated rule".into(),
        days: vec![Weekday::Monday],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::SpeedLimit(5_000_000),
        enabled: false,
    };

    let success = scheduler.update_rule(updated_rule);
    assert!(success);
    assert_eq!(scheduler.rules()[0].name, "Updated rule");
    assert_eq!(scheduler.rules()[0].days.len(), 1);
    assert!(!scheduler.rules()[0].enabled);

    // Try to update non-existent rule
    let non_existent = ScheduleRule {
        id: RuleId(99),
        name: "Non-existent".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(1, 0, 0).unwrap(),
        action: ScheduleAction::Pause,
        enabled: true,
    };

    let failed = scheduler.update_rule(non_existent);
    assert!(!failed);
    assert_eq!(scheduler.rules().len(), 1); // Still only 1 rule
}

#[test]
fn test_scheduler_set_rules() {
    let initial_rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Rule 1".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    }];

    let mut scheduler = Scheduler::new(initial_rules);
    assert_eq!(scheduler.rules().len(), 1);

    // Replace with new rules
    let new_rules = vec![
        ScheduleRule {
            id: RuleId(2),
            name: "New Rule 1".into(),
            days: vec![Weekday::Monday],
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            action: ScheduleAction::SpeedLimit(1_000_000),
            enabled: true,
        },
        ScheduleRule {
            id: RuleId(3),
            name: "New Rule 2".into(),
            days: vec![Weekday::Friday],
            start_time: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
            action: ScheduleAction::Unlimited,
            enabled: true,
        },
    ];

    scheduler.set_rules(new_rules);
    assert_eq!(scheduler.rules().len(), 2);
    assert_eq!(scheduler.rules()[0].id, RuleId(2));
    assert_eq!(scheduler.rules()[1].id, RuleId(3));
}

#[test]
fn test_scheduler_multiple_operations() {
    let mut scheduler = Scheduler::default();

    // Add three rules
    scheduler.add_rule(ScheduleRule {
        id: RuleId(1),
        name: "Rule 1".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    });

    scheduler.add_rule(ScheduleRule {
        id: RuleId(2),
        name: "Rule 2".into(),
        days: vec![Weekday::Monday, Weekday::Friday],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::SpeedLimit(2_000_000),
        enabled: true,
    });

    scheduler.add_rule(ScheduleRule {
        id: RuleId(3),
        name: "Rule 3".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
        action: ScheduleAction::Pause,
        enabled: true,
    });

    assert_eq!(scheduler.rules().len(), 3);

    // Remove middle rule
    scheduler.remove_rule(RuleId(2));
    assert_eq!(scheduler.rules().len(), 2);
    assert_eq!(scheduler.rules()[0].id, RuleId(1));
    assert_eq!(scheduler.rules()[1].id, RuleId(3));

    // Update remaining rule
    scheduler.update_rule(ScheduleRule {
        id: RuleId(3),
        name: "Updated Rule 3".into(),
        days: vec![Weekday::Saturday, Weekday::Sunday],
        start_time: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: false,
    });

    assert_eq!(scheduler.rules()[1].name, "Updated Rule 3");
    assert!(!scheduler.rules()[1].enabled);
}

#[test]
fn test_get_current_action_no_rules() {
    use chrono::Local;

    let scheduler = Scheduler::default();
    let now = Local::now();

    assert!(scheduler.get_current_action(now).is_none());
}

#[test]
fn test_get_current_action_disabled_rule() {
    use chrono::Local;

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Disabled rule".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
        action: ScheduleAction::Pause,
        enabled: false, // Disabled
    }];

    let scheduler = Scheduler::new(rules);
    let now = Local::now();

    // Disabled rule should not match
    assert!(scheduler.get_current_action(now).is_none());
}

#[test]
fn test_get_current_action_time_match() {
    use chrono::Local;

    // Create a specific time: 10:30 AM on a Monday
    let now = Local::now()
        .with_hour(10)
        .unwrap()
        .with_minute(30)
        .unwrap()
        .with_second(0)
        .unwrap();

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Morning rule".into(),
        days: vec![], // All days
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        action: ScheduleAction::SpeedLimit(1_000_000),
        enabled: true,
    }];

    let scheduler = Scheduler::new(rules);
    let action = scheduler.get_current_action(now);

    assert!(action.is_some());
    assert_eq!(action.unwrap(), ScheduleAction::SpeedLimit(1_000_000));
}

#[test]
fn test_get_current_action_time_no_match() {
    use chrono::Local;

    // Create a specific time: 2:00 PM
    let now = Local::now()
        .with_hour(14)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Morning rule".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        action: ScheduleAction::SpeedLimit(1_000_000),
        enabled: true,
    }];

    let scheduler = Scheduler::new(rules);
    let action = scheduler.get_current_action(now);

    // 2:00 PM is outside 9:00-12:00 range
    assert!(action.is_none());
}

#[test]
fn test_get_current_action_day_match() {
    use chrono::Local;

    let now = Local::now()
        .with_hour(10)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();

    let current_weekday = Weekday::from_chrono(now.weekday());

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Today only".into(),
        days: vec![current_weekday], // Only matches today
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    }];

    let scheduler = Scheduler::new(rules);
    let action = scheduler.get_current_action(now);

    assert!(action.is_some());
    assert_eq!(action.unwrap(), ScheduleAction::Unlimited);
}

#[test]
fn test_get_current_action_day_no_match() {
    use chrono::Local;

    let now = Local::now()
        .with_hour(10)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();

    let current_weekday = Weekday::from_chrono(now.weekday());

    // Pick a different weekday
    let different_weekday = match current_weekday {
        Weekday::Monday => Weekday::Tuesday,
        _ => Weekday::Monday,
    };

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Different day".into(),
        days: vec![different_weekday], // Not today
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::Pause,
        enabled: true,
    }];

    let scheduler = Scheduler::new(rules);
    let action = scheduler.get_current_action(now);

    // Wrong day, should not match
    assert!(action.is_none());
}

#[test]
fn test_get_current_action_empty_days_matches_all() {
    use chrono::Local;

    let now = Local::now()
        .with_hour(10)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "All days".into(),
        days: vec![], // Empty = all days
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::SpeedLimit(5_000_000),
        enabled: true,
    }];

    let scheduler = Scheduler::new(rules);
    let action = scheduler.get_current_action(now);

    // Empty days should match any day
    assert!(action.is_some());
    assert_eq!(action.unwrap(), ScheduleAction::SpeedLimit(5_000_000));
}

#[test]
fn test_get_current_action_first_match_wins() {
    use chrono::Local;

    let now = Local::now()
        .with_hour(10)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();

    let rules = vec![
        ScheduleRule {
            id: RuleId(1),
            name: "First rule".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            action: ScheduleAction::SpeedLimit(1_000_000),
            enabled: true,
        },
        ScheduleRule {
            id: RuleId(2),
            name: "Second rule (should not match)".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            action: ScheduleAction::Unlimited,
            enabled: true,
        },
    ];

    let scheduler = Scheduler::new(rules);
    let action = scheduler.get_current_action(now);

    // First matching rule should win
    assert!(action.is_some());
    assert_eq!(action.unwrap(), ScheduleAction::SpeedLimit(1_000_000));
}

#[test]
fn test_get_current_action_boundary_start_inclusive() {
    use chrono::Local;

    // Time exactly at start_time (9:00:00)
    let now = Local::now()
        .with_hour(9)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Boundary test".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::Pause,
        enabled: true,
    }];

    let scheduler = Scheduler::new(rules);
    let action = scheduler.get_current_action(now);

    // Start time is inclusive (>=)
    assert!(action.is_some());
    assert_eq!(action.unwrap(), ScheduleAction::Pause);
}

#[test]
fn test_get_current_action_boundary_end_exclusive() {
    use chrono::Local;

    // Time exactly at end_time (17:00:00)
    let now = Local::now()
        .with_hour(17)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Boundary test".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::SpeedLimit(1_000_000),
        enabled: true,
    }];

    let scheduler = Scheduler::new(rules);
    let action = scheduler.get_current_action(now);

    // End time is exclusive (<)
    assert!(action.is_none());
}

#[test]
fn test_get_current_action_all_action_types() {
    use chrono::Local;

    let now = Local::now()
        .with_hour(10)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();

    // Test SpeedLimit
    let scheduler = Scheduler::new(vec![ScheduleRule {
        id: RuleId(1),
        name: "Speed limit".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::SpeedLimit(2_000_000),
        enabled: true,
    }]);
    assert_eq!(
        scheduler.get_current_action(now),
        Some(ScheduleAction::SpeedLimit(2_000_000))
    );

    // Test Unlimited
    let scheduler = Scheduler::new(vec![ScheduleRule {
        id: RuleId(2),
        name: "Unlimited".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    }]);
    assert_eq!(
        scheduler.get_current_action(now),
        Some(ScheduleAction::Unlimited)
    );

    // Test Pause
    let scheduler = Scheduler::new(vec![ScheduleRule {
        id: RuleId(3),
        name: "Pause".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::Pause,
        enabled: true,
    }]);
    assert_eq!(
        scheduler.get_current_action(now),
        Some(ScheduleAction::Pause)
    );
}

#[test]
fn test_get_current_action_complex_scenario() {
    use chrono::Local;

    let now = Local::now()
        .with_hour(14)
        .unwrap() // 2 PM
        .with_minute(30)
        .unwrap()
        .with_second(0)
        .unwrap();

    let _current_weekday = Weekday::from_chrono(now.weekday());

    let rules = vec![
        // Rule 1: Disabled, should be ignored
        ScheduleRule {
            id: RuleId(1),
            name: "Disabled".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
            action: ScheduleAction::Pause,
            enabled: false,
        },
        // Rule 2: Wrong time window, should be ignored
        ScheduleRule {
            id: RuleId(2),
            name: "Morning only".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            action: ScheduleAction::Unlimited,
            enabled: true,
        },
        // Rule 3: Wrong day, should be ignored
        ScheduleRule {
            id: RuleId(3),
            name: "Wrong day".into(),
            days: vec![Weekday::Sunday], // Unlikely to match
            start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
            action: ScheduleAction::SpeedLimit(100),
            enabled: true,
        },
        // Rule 4: Should match (all days, correct time)
        ScheduleRule {
            id: RuleId(4),
            name: "Afternoon".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
            action: ScheduleAction::SpeedLimit(3_000_000),
            enabled: true,
        },
        // Rule 5: Also matches but should not be returned (first match wins)
        ScheduleRule {
            id: RuleId(5),
            name: "Also matches".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(16, 0, 0).unwrap(),
            action: ScheduleAction::Pause,
            enabled: true,
        },
    ];

    let scheduler = Scheduler::new(rules);
    let action = scheduler.get_current_action(now);

    // Should match Rule 4 (first enabled rule with matching time and day)
    assert!(action.is_some());
    assert_eq!(action.unwrap(), ScheduleAction::SpeedLimit(3_000_000));
}

// ============================================================================
// Time-based rule transition tests
// ============================================================================

#[test]
fn test_time_transition_entering_rule_window() {
    use chrono::Local;

    // Rule: 9:00-17:00 with speed limit
    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Work hours".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::SpeedLimit(1_000_000),
        enabled: true,
    }];
    let scheduler = Scheduler::new(rules);

    // Test 1 second before rule starts (8:59:59)
    let before = Local::now()
        .with_hour(8)
        .unwrap()
        .with_minute(59)
        .unwrap()
        .with_second(59)
        .unwrap();
    assert!(scheduler.get_current_action(before).is_none());

    // Test exactly at start time (9:00:00)
    let at_start = Local::now()
        .with_hour(9)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(at_start).unwrap(),
        ScheduleAction::SpeedLimit(1_000_000)
    );

    // Test 1 second after start (9:00:01)
    let after = Local::now()
        .with_hour(9)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(1)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(after).unwrap(),
        ScheduleAction::SpeedLimit(1_000_000)
    );
}

#[test]
fn test_time_transition_exiting_rule_window() {
    use chrono::Local;

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Work hours".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        action: ScheduleAction::SpeedLimit(1_000_000),
        enabled: true,
    }];
    let scheduler = Scheduler::new(rules);

    // Test 1 second before rule ends (16:59:59)
    let before = Local::now()
        .with_hour(16)
        .unwrap()
        .with_minute(59)
        .unwrap()
        .with_second(59)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(before).unwrap(),
        ScheduleAction::SpeedLimit(1_000_000)
    );

    // Test exactly at end time (17:00:00) - should NOT match (exclusive)
    let at_end = Local::now()
        .with_hour(17)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert!(scheduler.get_current_action(at_end).is_none());

    // Test 1 second after end (17:00:01)
    let after = Local::now()
        .with_hour(17)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(1)
        .unwrap();
    assert!(scheduler.get_current_action(after).is_none());
}

#[test]
fn test_time_transition_between_sequential_rules() {
    use chrono::Local;

    // Two back-to-back rules: 9:00-12:00, then 12:00-17:00
    let rules = vec![
        ScheduleRule {
            id: RuleId(1),
            name: "Morning".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            action: ScheduleAction::SpeedLimit(500_000),
            enabled: true,
        },
        ScheduleRule {
            id: RuleId(2),
            name: "Afternoon".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            action: ScheduleAction::SpeedLimit(2_000_000),
            enabled: true,
        },
    ];
    let scheduler = Scheduler::new(rules);

    // Test morning rule (11:59:59)
    let morning = Local::now()
        .with_hour(11)
        .unwrap()
        .with_minute(59)
        .unwrap()
        .with_second(59)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(morning).unwrap(),
        ScheduleAction::SpeedLimit(500_000)
    );

    // Test exactly at transition (12:00:00) - should match afternoon
    let transition = Local::now()
        .with_hour(12)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(transition).unwrap(),
        ScheduleAction::SpeedLimit(2_000_000)
    );

    // Test afternoon rule (12:00:01)
    let afternoon = Local::now()
        .with_hour(12)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(1)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(afternoon).unwrap(),
        ScheduleAction::SpeedLimit(2_000_000)
    );
}

#[test]
fn test_time_transition_one_minute_window() {
    use chrono::Local;

    // Very short rule: 14:30:00 - 14:31:00
    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "One minute window".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(14, 30, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(14, 31, 0).unwrap(),
        action: ScheduleAction::Pause,
        enabled: true,
    }];
    let scheduler = Scheduler::new(rules);

    // Before window
    let before = Local::now()
        .with_hour(14)
        .unwrap()
        .with_minute(29)
        .unwrap()
        .with_second(59)
        .unwrap();
    assert!(scheduler.get_current_action(before).is_none());

    // At start
    let at_start = Local::now()
        .with_hour(14)
        .unwrap()
        .with_minute(30)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(at_start).unwrap(),
        ScheduleAction::Pause
    );

    // Middle of window
    let middle = Local::now()
        .with_hour(14)
        .unwrap()
        .with_minute(30)
        .unwrap()
        .with_second(30)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(middle).unwrap(),
        ScheduleAction::Pause
    );

    // At end (exclusive)
    let at_end = Local::now()
        .with_hour(14)
        .unwrap()
        .with_minute(31)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert!(scheduler.get_current_action(at_end).is_none());
}

#[test]
fn test_time_transition_midnight_boundary_simple() {
    use chrono::Local;

    // Rule that does NOT cross midnight: 22:00 - 23:59
    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Late evening".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    }];
    let scheduler = Scheduler::new(rules);

    // Before midnight, in window
    let before_midnight = Local::now()
        .with_hour(23)
        .unwrap()
        .with_minute(30)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(before_midnight).unwrap(),
        ScheduleAction::Unlimited
    );

    // After midnight - should NOT match (new day)
    let after_midnight = Local::now()
        .with_hour(0)
        .unwrap()
        .with_minute(30)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert!(scheduler.get_current_action(after_midnight).is_none());
}

#[test]
fn test_overlapping_rules_priority_order() {
    use chrono::Local;

    // Three overlapping rules with different priorities
    let rules = vec![
        // Rule 1: General all-day rule (lowest priority, should match last)
        ScheduleRule {
            id: RuleId(1),
            name: "General all day".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
            action: ScheduleAction::SpeedLimit(1_000_000),
            enabled: true,
        },
        // Rule 2: Work hours override (medium priority)
        ScheduleRule {
            id: RuleId(2),
            name: "Work hours".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            action: ScheduleAction::SpeedLimit(500_000),
            enabled: true,
        },
        // Rule 3: Lunch break (highest priority, most specific)
        ScheduleRule {
            id: RuleId(3),
            name: "Lunch break".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
            action: ScheduleAction::Unlimited,
            enabled: true,
        },
    ];
    let scheduler = Scheduler::new(rules);

    // Early morning (8:00) - should match Rule 1 (general)
    let morning = Local::now()
        .with_hour(8)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(morning).unwrap(),
        ScheduleAction::SpeedLimit(1_000_000)
    );

    // Work hours (10:00) - should match Rule 1 (first match wins!)
    let work = Local::now()
        .with_hour(10)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(work).unwrap(),
        ScheduleAction::SpeedLimit(1_000_000) // Rule 1 wins!
    );

    // Lunch (12:30) - should match Rule 1 (first match wins!)
    let lunch = Local::now()
        .with_hour(12)
        .unwrap()
        .with_minute(30)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(lunch).unwrap(),
        ScheduleAction::SpeedLimit(1_000_000) // Rule 1 wins!
    );
}

#[test]
fn test_action_type_transitions() {
    use chrono::Local;

    // Rules with different action types in sequence
    let rules = vec![
        ScheduleRule {
            id: RuleId(1),
            name: "Morning speed limit".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            action: ScheduleAction::SpeedLimit(500_000),
            enabled: true,
        },
        ScheduleRule {
            id: RuleId(2),
            name: "Work pause".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            action: ScheduleAction::Pause,
            enabled: true,
        },
        ScheduleRule {
            id: RuleId(3),
            name: "Evening unlimited".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
            action: ScheduleAction::Unlimited,
            enabled: true,
        },
    ];
    let scheduler = Scheduler::new(rules);

    // Morning: SpeedLimit
    let morning = Local::now()
        .with_hour(7)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(morning).unwrap(),
        ScheduleAction::SpeedLimit(500_000)
    );

    // Work: Pause
    let work = Local::now()
        .with_hour(12)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(work).unwrap(),
        ScheduleAction::Pause
    );

    // Evening: Unlimited
    let evening = Local::now()
        .with_hour(20)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(evening).unwrap(),
        ScheduleAction::Unlimited
    );

    // Night: None
    let night = Local::now()
        .with_hour(23)
        .unwrap()
        .with_minute(30)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert!(scheduler.get_current_action(night).is_none());
}

#[test]
fn test_specific_day_vs_all_days_priority() {
    use chrono::{Datelike, Local};

    // Find a Monday for testing
    let base = Local::now();
    let mut monday = base;
    while monday.weekday() != chrono::Weekday::Mon {
        monday += chrono::Duration::days(1);
    }

    let rules = vec![
        // General rule (all days)
        ScheduleRule {
            id: RuleId(1),
            name: "All days slow".into(),
            days: vec![],
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            action: ScheduleAction::SpeedLimit(1_000_000),
            enabled: true,
        },
        // Specific Monday rule
        ScheduleRule {
            id: RuleId(2),
            name: "Monday fast".into(),
            days: vec![Weekday::Monday],
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            action: ScheduleAction::Unlimited,
            enabled: true,
        },
    ];
    let scheduler = Scheduler::new(rules);

    let monday_work = monday
        .with_hour(12)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();

    // First match wins - general rule matches first
    assert_eq!(
        scheduler.get_current_action(monday_work).unwrap(),
        ScheduleAction::SpeedLimit(1_000_000)
    );
}

#[test]
fn test_minute_boundary_precision() {
    use chrono::Local;

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "10:30 start".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(10, 30, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(11, 30, 0).unwrap(),
        action: ScheduleAction::Pause,
        enabled: true,
    }];
    let scheduler = Scheduler::new(rules);

    // 10:29:59 - should NOT match
    let before = Local::now()
        .with_hour(10)
        .unwrap()
        .with_minute(29)
        .unwrap()
        .with_second(59)
        .unwrap();
    assert!(scheduler.get_current_action(before).is_none());

    // 10:30:00 - should match
    let at_boundary = Local::now()
        .with_hour(10)
        .unwrap()
        .with_minute(30)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(at_boundary).unwrap(),
        ScheduleAction::Pause
    );

    // 10:30:01 - should match
    let after_boundary = Local::now()
        .with_hour(10)
        .unwrap()
        .with_minute(30)
        .unwrap()
        .with_second(1)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(after_boundary).unwrap(),
        ScheduleAction::Pause
    );

    // 10:30:30 - should match (middle of minute)
    let mid_minute = Local::now()
        .with_hour(10)
        .unwrap()
        .with_minute(30)
        .unwrap()
        .with_second(30)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(mid_minute).unwrap(),
        ScheduleAction::Pause
    );

    // 11:29:59 - should match
    let before_end = Local::now()
        .with_hour(11)
        .unwrap()
        .with_minute(29)
        .unwrap()
        .with_second(59)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(before_end).unwrap(),
        ScheduleAction::Pause
    );

    // 11:30:00 - should NOT match (exclusive end)
    let at_end = Local::now()
        .with_hour(11)
        .unwrap()
        .with_minute(30)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert!(scheduler.get_current_action(at_end).is_none());
}

// ============================================================================
// Midnight-crossing rule tests (start_time > end_time, OR-logic branch)
// ============================================================================

#[test]
fn test_midnight_crossing_rule_matches_before_midnight() {
    use chrono::Local;

    // Rule: 22:00 → 06:00 (crosses midnight)
    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Night unlimited".into(),
        days: vec![], // All days
        start_time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    }];
    let scheduler = Scheduler::new(rules);

    // 23:00 is after start (22:00), should match via `time >= start_time`
    let at_23 = Local::now()
        .with_hour(23)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(at_23),
        Some(ScheduleAction::Unlimited),
        "23:00 should match a 22:00→06:00 rule (before-midnight side)"
    );
}

#[test]
fn test_midnight_crossing_rule_matches_after_midnight() {
    use chrono::Local;

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Night unlimited".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    }];
    let scheduler = Scheduler::new(rules);

    // 03:00 is before end (06:00), should match via `time < end_time`
    let at_03 = Local::now()
        .with_hour(3)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(at_03),
        Some(ScheduleAction::Unlimited),
        "03:00 should match a 22:00→06:00 rule (after-midnight side)"
    );
}

#[test]
fn test_midnight_crossing_rule_does_not_match_daytime() {
    use chrono::Local;

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Night unlimited".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    }];
    let scheduler = Scheduler::new(rules);

    // 12:00 is in the gap: after end (06:00) and before start (22:00)
    let at_12 = Local::now()
        .with_hour(12)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert!(
        scheduler.get_current_action(at_12).is_none(),
        "12:00 should NOT match a 22:00→06:00 rule"
    );
}

#[test]
fn test_midnight_crossing_rule_boundary_start_inclusive() {
    use chrono::Local;

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Night unlimited".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    }];
    let scheduler = Scheduler::new(rules);

    // Exactly at start_time (22:00:00) — should match (>=)
    let at_start = Local::now()
        .with_hour(22)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert_eq!(
        scheduler.get_current_action(at_start),
        Some(ScheduleAction::Unlimited),
        "exactly 22:00 should match a 22:00→06:00 rule (start is inclusive)"
    );
}

#[test]
fn test_midnight_crossing_rule_boundary_end_exclusive() {
    use chrono::Local;

    let rules = vec![ScheduleRule {
        id: RuleId(1),
        name: "Night unlimited".into(),
        days: vec![],
        start_time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    }];
    let scheduler = Scheduler::new(rules);

    // Exactly at end_time (06:00:00) — should NOT match (<, exclusive)
    let at_end = Local::now()
        .with_hour(6)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap();
    assert!(
        scheduler.get_current_action(at_end).is_none(),
        "exactly 06:00 should NOT match a 22:00→06:00 rule (end is exclusive)"
    );
}
