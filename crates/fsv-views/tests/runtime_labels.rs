//! Python `936e767` (`test_shifted_tenant.test_priority_impact_labels_come_from_runtime_choices`):
//! the classic list shows the tenant's own priority labels.

use fsv_core::resource::TICKETS;
use serde_json::json;

#[test]
fn list_rows_use_runtime_priority_and_status_labels() {
    let sch = json!({"fields": [
        {"name": "priority", "choices": [{"id": 1, "value": "P4"}, {"id": 2, "value": "P3"},
                                         {"id": 3, "value": "P2"}, {"id": 4, "value": "P1"}]},
        {"name": "status", "choices": [{"id": 2, "value": "Offen"}, {"id": 3, "value": "Wartend"},
                                       {"id": 12, "value": "Abgebrochen"}, {"id": 4, "value": "Gelöst"},
                                       {"id": 5, "value": "Erledigt"}, {"id": 13, "value": "In Arbeit"}]},
    ]});
    let item = json!({"id": 7, "display_id": 7, "human_display_id": "IT-7", "status": 4, "priority": 1,
                      "subject": "Globex"});
    let row = &fsv_views::render::list_rows(&[item], &TICKETS, Some(&sch))[0];
    assert_eq!(row["priority"], "P4");
    assert_eq!(row["id"], "IT-7");
    assert_eq!(row["status"], "Gelöst");
}
