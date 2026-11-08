use super::*;

fn step(changed: Option<bool>, counts: Option<Changes>) -> StepOutcome {
    StepOutcome {
        changed,
        changes: counts,
        ..StepOutcome::new(Some("x".into()), StepState::Succeeded)
    }
}

fn outcome(steps: Vec<StepOutcome>) -> Outcome {
    Outcome {
        task: Task::Cleanup,
        manager: "apt".into(),
        dry_run: false,
        steps,
    }
}

const ONE_REMOVED: Changes = Changes {
    upgraded: 0,
    installed: 0,
    removed: 1,
};

#[test]
fn any_change_counts() {
    let outcome = outcome(vec![step(None, None), step(Some(true), Some(ONE_REMOVED))]);
    assert_eq!(outcome.changed(), Some(true));
    assert_eq!(outcome.changes(), None);
}

#[test]
fn unknown_beats_nothing() {
    let outcome = outcome(vec![
        step(Some(false), Some(Changes::default())),
        step(None, None),
    ]);
    assert_eq!(outcome.changed(), None);
}

#[test]
fn nothing_changed_when_every_step_says_so() {
    let outcome = outcome(vec![
        step(Some(false), Some(Changes::default())),
        StepOutcome::note("n/a"),
    ]);
    assert_eq!(outcome.changed(), Some(false));
    assert_eq!(outcome.changes(), Some(Changes::default()));
}

#[test]
fn changes_sum_across_steps() {
    let outcome = outcome(vec![
        step(Some(true), Some(ONE_REMOVED)),
        step(Some(true), Some(ONE_REMOVED)),
    ]);
    assert_eq!(outcome.changes().map(|changes| changes.removed), Some(2));
}

#[test]
fn planned_steps_know_nothing() {
    assert_eq!(
        outcome(vec![StepOutcome::planned("x".into())]).changed(),
        None
    );
}

#[test]
fn tail_keeps_the_last_lines() {
    let text = (1..=100)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let tail = tail(&text);
    assert_eq!(tail.lines().count(), TAIL_LINES);
    assert!(tail.starts_with("61\n"));
    assert!(tail.ends_with("100"));
    assert_eq!(super::tail("short"), "short");
}

#[test]
fn step_state_flattens_into_the_step() {
    let json = serde_json::to_value(StepOutcome::skipped("rm".into(), "none".into())).unwrap();
    assert_eq!(json["state"], "skipped");
    assert_eq!(json["reason"], "none");
    assert_eq!(json["command"], "rm");
    let back: StepOutcome = serde_json::from_value(json).unwrap();
    assert_eq!(
        back.state,
        StepState::Skipped {
            reason: "none".into()
        }
    );
}

#[test]
fn tasks_name_their_command() {
    assert_eq!(Task::Refresh.as_str(), "refresh");
    assert_eq!(Task::FullUpgrade.as_str(), "upgrade --full");
    assert_eq!(Task::Cleanup.as_str(), "cleanup");
}
