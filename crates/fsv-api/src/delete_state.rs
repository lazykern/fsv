//! Record-state tokens for delete plans (`journal.delete_state_tokens`).

use serde_json::{Map, Value, json};

use crate::service::Journal;

/// One token: the raw server `updated_at` and the journaled restore count.
pub(crate) fn state_token(row: &Map<String, Value>, restores: usize) -> Value {
    json!({
        "server_version": row.get("updated_at").cloned().unwrap_or(Value::Null),
        "restores": restores,
    })
}

/// Port of journal.delete_state_tokens: record-state tokens that move when
/// a record is restored, for delete plans.
///
/// Stable snapshots drop `updated_at`, so a record restored to the same
/// business state would plan the same delete hash and a legitimate delete
/// after a restore would be refused as a replay. Each token binds the raw
/// server `updated_at` read at review time (the server bumps it on restore)
/// and how many restores of this exact target fsv has journaled as answered
/// (monotonic, even when the server keeps `updated_at`). A true replay
/// against an unchanged record keeps the token, so the guard still refuses
/// it; a `started` without an answer is unchanged and stays non-replayable.
/// The key is not `updated_at`: snapshot hashing drops that name.
pub fn delete_state_tokens(
    journal: &dyn Journal,
    rows: &[Map<String, Value>],
    targets: &[i64],
    restore_actions: &[&str],
) -> Vec<Value> {
    let kinds: Vec<String> = restore_actions
        .iter()
        .map(|a| format!("plan:{a}"))
        .collect();
    let events = if kinds.is_empty() {
        Vec::new()
    } else {
        journal.read()
    };
    let restores = |target: i64| {
        events
            .iter()
            .filter(|e| {
                e.get("event").and_then(Value::as_str) == Some("done")
                    && e.get("kind")
                        .and_then(Value::as_str)
                        .is_some_and(|k| kinds.iter().any(|x| x == k))
                    && e.get("target") == Some(&json!(target))
            })
            .count()
    };
    rows.iter()
        .zip(targets)
        .map(|(row, target)| state_token(row, restores(*target)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::obj;
    use crate::service::testing::MemJournal;

    #[test]
    fn counts_answered_restores_of_the_exact_target_only() {
        let j = MemJournal::default();
        let row = obj(json!({"id": 71, "updated_at": "2030-01-01T00:00:00Z"}));
        let rows = [row.clone(), obj(json!({"id": 72}))];
        assert_eq!(
            delete_state_tokens(&j, &rows, &[71, 72], &["ticket_restore"]),
            [
                json!({"server_version": "2030-01-01T00:00:00Z", "restores": 0}),
                json!({"server_version": null, "restores": 0}),
            ]
        );
        j.record_plan(
            "done",
            &"0".repeat(64),
            "ticket_restore",
            Some(&json!(71)),
            None,
        );
        j.record_plan(
            "started",
            &"1".repeat(64),
            "ticket_restore",
            Some(&json!(71)),
            None,
        );
        j.record_plan(
            "done",
            &"2".repeat(64),
            "problem_restore",
            Some(&json!(71)),
            None,
        );
        j.record_plan(
            "done",
            &"3".repeat(64),
            "ticket_restore",
            Some(&json!(72)),
            None,
        );
        j.record_plan(
            "done",
            &"4".repeat(64),
            "ticket_restore",
            Some(&json!(72)),
            None,
        );
        let got = delete_state_tokens(&j, &rows, &[71, 72], &["ticket_restore"]);
        assert_eq!(got[0]["restores"], json!(1));
        assert_eq!(got[1]["restores"], json!(2));
        assert_eq!(
            delete_state_tokens(&j, &rows[..1], &[71], &[])[0]["restores"],
            json!(0)
        );
    }
}
