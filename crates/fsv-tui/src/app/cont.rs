//! Modal continuations.

use super::*;

impl App {
    pub(super) fn resume(&mut self, cont: Cont, result: ModalResult) {
        use ModalResult as R;
        match (cont, result) {
            (Cont::None, _) => {}
            (Cont::ViewPicked { res, views }, R::Choice(i)) => {
                let Some(view) = i.as_u64().and_then(|i| views.get(i as usize)).cloned() else {
                    return;
                };
                let msg = format!("[dim]loading {}...[/]", escape(&view.name));
                self.view.insert(res.name, view);
                self.active_filters.clear();
                self.reload_list(&msg);
            }
            (Cont::StatusPicked { item }, R::Status(id, _label)) => {
                if item.res.name == "tickets" {
                    self.prepare_ticket_property(item, "status".into(), id);
                } else {
                    self.prepare_change_status(item, id);
                }
            }
            (Cont::StatusConfirmed { item, id, label }, R::Bool(true)) => {
                self.do_write(WriteKind::Status(id, label), item)
            }
            (Cont::NoteComposed { item, reply }, R::NoteOptions(body, public))
                if item.res.name == "tickets" =>
            {
                self.ticket_conversation_options(item, body, public, reply, Default::default())
            }
            (Cont::NoteComposed { item, reply }, R::Note(body, public)) => {
                if item.res.name == "tickets" && reply {
                    self.prepare_ticket_conversation(item, body, public, reply, Default::default());
                    return;
                }
                if item.res.name == "tickets" && !reply {
                    self.review_ticket_note(item, body, public);
                    return;
                }
                if item.res.name == "problems" && !reply {
                    self.prepare_problem_note(item, body);
                    return;
                }
                let _ = public;
                self.show_failure(
                    "No shared reviewed sender for this operation; nothing sent.".to_string(),
                );
            }
            (Cont::NoteConfirmed { item, body, public }, R::Bool(true)) => {
                self.do_write(WriteKind::Note(body, public), item)
            }
            (Cont::TicketNoteReviewed { plan }, R::Bool(true)) => {
                if plan.public {
                    let lines = vec![
                        "This is a public note visible to the customer, not a reply.".to_string(),
                        "No explicit notification recipients. Tenant workflows may still notify people.".to_string(),
                    ];
                    self.push(
                        Modal::confirm("Confirm PUBLIC visibility", lines),
                        Cont::TicketNotePublic { plan },
                    );
                } else {
                    self.send_ticket_note(plan, false);
                }
            }
            (Cont::TicketNotePublic { plan }, R::Bool(true)) => self.send_ticket_note(plan, true),
            (Cont::ActionMenu { item }, R::Choice(kind)) => {
                self.prepare_action(&fsv_py::py_str(&kind), item)
            }
            (Cont::LinkQuery { item }, R::Text(q)) => {
                self.load_action_targets("link".into(), item, q)
            }
            (Cont::TaskTitle { item }, R::Text(title)) => self.task_titled(item, title),
            (Cont::TaskOwnerMode { item, title }, R::Choice(mode)) => {
                if self.read_only {
                    return;
                }
                if mode == "none" {
                    self.change_task_options(item, title, None, Vec::new());
                } else {
                    self.push(
                        Modal::value(
                            "Explicit task owner user ID or exact email (no implicit self)",
                            "",
                        ),
                        Cont::TaskOwner { item, title },
                    );
                }
            }
            (Cont::TaskOwner { item, title }, R::Text(owner)) => {
                self.change_task_options(item, title, Some(owner), Vec::new())
            }
            (
                Cont::TaskOptions {
                    item,
                    title,
                    owner,
                    values,
                },
                R::Choice(key),
            ) => {
                if self.read_only {
                    return;
                }
                let key = fsv_py::py_str(&key);
                match key.as_str() {
                    "review" => self.prepare_change_task(item, title, owner, values),
                    "clear" => self.change_task_options(item, title, owner, Vec::new()),
                    "status" | "notify_before" => {
                        self.load_task_create_choices(item, title, owner, values, key);
                    }
                    _ => {
                        let label = TASK_FIELDS
                            .iter()
                            .find(|(k, _)| *k == key)
                            .map_or("", |(_, l)| l);
                        let cur = values
                            .iter()
                            .find(|(k, _)| *k == key)
                            .map_or("", |(_, v)| v.as_str())
                            .to_string();
                        self.push(
                            Modal::value(label, &cur),
                            Cont::TaskOptionValue {
                                item,
                                title,
                                owner,
                                values,
                                key,
                            },
                        );
                    }
                }
            }
            (
                Cont::TaskOptionValue {
                    item,
                    title,
                    owner,
                    mut values,
                    key,
                },
                R::Text(v) | R::Choice(Value::String(v)),
            ) => {
                if self.read_only {
                    return;
                }
                match values.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = v,
                    None => values.push((key, v)),
                }
                self.change_task_options(item, title, owner, values);
            }
            (Cont::TaskReviewed { item, plan }, R::Bool(true)) => self.send_change_task(item, plan),
            (Cont::TaskMutationReviewed { item, plan }, R::Bool(true)) => {
                self.send_task_mutation(item, plan)
            }
            (Cont::ProblemNoteReviewed { item, plan }, R::Bool(true)) => {
                self.send_problem_note(item, plan)
            }
            (Cont::ChangeStatusReviewed { item, plan }, R::Bool(true)) => {
                self.send_change_status(item, plan)
            }
            (Cont::ActionTarget { kind, item }, R::Choice(target)) => {
                if self.read_only {
                    return;
                }
                match kind.as_str() {
                    "task-update" => self.load_task_editor(item, target),
                    "download" => self.push(
                        Modal::value("Download destination directory", "."),
                        Cont::DownloadDest { item, target },
                    ),
                    _ => self.confirm_action(kind, item, target, Map::new()),
                }
            }
            (
                Cont::TaskField {
                    item,
                    target,
                    current,
                    fields,
                },
                R::Choice(field),
            ) => {
                let field = fsv_py::py_str(&field);
                let title = format!("Task {}: {field}", fget(&target, "id", ""));
                let cur = current
                    .get(&field)
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default();
                self.push(
                    Modal::value(&title, &cur),
                    Cont::TaskFieldValue {
                        item,
                        target,
                        current,
                        fields,
                        field,
                    },
                );
            }
            (
                Cont::TaskFieldValue {
                    item,
                    target,
                    current,
                    fields,
                    field,
                },
                R::Text(value),
            ) => match actions::task_patch(&current, &field, &value, &fields) {
                Err(e) => self.show_failure(e),
                Ok(body) if body.is_empty() => {
                    self.notify("No change; nothing sent", Severity::Information)
                }
                Ok(body) => {
                    let before: Map<String, Value> = body
                        .keys()
                        .map(|k| (k.clone(), current.get(k).cloned().unwrap_or(Value::Null)))
                        .collect();
                    let mut t = target;
                    if let Value::Object(o) = &mut t {
                        o.insert("_before".into(), Value::Object(before));
                    }
                    self.confirm_action("task-update".into(), item, t, body);
                }
            },
            (Cont::DownloadDest { item, target }, R::Text(path)) => {
                self.prepare_download(item, target, path, None)
            }
            (
                Cont::DownloadCollision {
                    item,
                    target,
                    directory,
                    name,
                },
                R::Choice(action),
            ) => {
                if action == "rename" && !self.read_only {
                    self.push(
                        Modal::value("Save as filename (no directories)", &name),
                        Cont::DownloadRename {
                            item,
                            target,
                            directory,
                        },
                    );
                }
            }
            (
                Cont::DownloadRename {
                    item,
                    target,
                    directory,
                },
                R::Text(name),
            ) => self.prepare_download(item, target, directory, Some(name)),
            (Cont::ApprovalName { item }, R::Text(name)) => {
                if self.read_only {
                    return;
                }
                self.push(
                    Modal::value("Approval message", ""),
                    Cont::ApprovalMessage { item, name },
                );
            }
            (Cont::ApprovalMessage { item, name }, R::Text(message)) => {
                if self.read_only {
                    return;
                }
                self.push(
                    Modal::value("Explicit approver IDs or exact emails, comma separated", ""),
                    Cont::ApprovalRecipients {
                        item,
                        name,
                        message,
                    },
                );
            }
            (
                Cont::ApprovalRecipients {
                    item,
                    name,
                    message,
                },
                R::Text(recipients),
            ) => {
                if self.read_only {
                    return;
                }
                let rules = site::approval_rules()
                    .into_iter()
                    .map(|(k, l)| (json!(k), l.to_string()))
                    .collect();
                self.push(
                    Modal::choices("Approval rule", rules),
                    Cont::ApprovalRule {
                        item,
                        name,
                        message,
                        recipients,
                    },
                );
            }
            (
                Cont::ApprovalRule {
                    item,
                    name,
                    message,
                    recipients,
                },
                R::Choice(rule),
            ) => self.prepare_approval_request(
                item,
                name,
                message,
                recipients,
                fsv_py::py_str(&rule),
            ),
            (Cont::ApprovalTarget { kind, item }, R::Choice(row)) => {
                if self.read_only {
                    return;
                }
                match kind.as_str() {
                    "approval-groups" => self.push(
                        Modal::outcome("Approval group details (read only)", &json_pretty(&row)),
                        Cont::None,
                    ),
                    "approval-cancel" => self.prepare_approval_action(item, row, "cancel", None),
                    _ => {
                        let ops = vec![
                            (json!("approve"), "Approve".to_string()),
                            (json!("reject"), "Reject".to_string()),
                        ];
                        self.push(
                            Modal::choices("Approval response", ops),
                            Cont::ApprovalOp { item, row },
                        );
                    }
                }
            }
            (Cont::ApprovalOp { item, row }, R::Choice(op)) => {
                if self.read_only {
                    return;
                }
                let op = fsv_py::py_str(&op);
                self.push(
                    Modal::value_with(
                        "Remark (required for reject; optional for approve)",
                        "",
                        op == "approve",
                    ),
                    Cont::ApprovalRemark { item, row, op },
                );
            }
            (Cont::ApprovalRemark { item, row, op }, R::Text(remark)) => {
                let remark = (!remark.is_empty()).then_some(remark);
                self.prepare_approval_action(item, row, &op, remark.as_deref());
            }
            (Cont::ApprovalReviewed { item, plan }, R::Bool(true)) => {
                self.send_approval(item, plan)
            }
            (
                Cont::ActionConfirmed {
                    kind,
                    item,
                    target,
                    body,
                    plan,
                },
                R::Bool(true),
            ) => self.execute_action(kind, item, target, body, plan),
            (Cont::CreateChoice, R::Choice(kind)) => self.create_choice(&fsv_py::py_str(&kind)),
            (Cont::CatalogCategory { items, categories }, R::Choice(category)) => {
                if self.read_only {
                    return;
                }
                let ids: Option<Vec<Value>> = if category == "all" {
                    None
                } else {
                    Some(
                        self.backend
                            .resolve_category(&categories, &fsv_py::py_str(&category))
                            .iter()
                            .filter_map(|c| c.get("id").cloned())
                            .collect(),
                    )
                };
                let rows = self.backend.filter_catalog_items(&items, ids.as_deref());
                let name_of = |cid: Option<&Value>| {
                    categories
                        .iter()
                        .find(|c| c.get("id") == cid)
                        .map_or(String::new(), |c| fget(c, "name", ""))
                };
                let choices = rows
                    .iter()
                    .filter(|r| r.get("display_id").is_some_and(|d| !d.is_null()))
                    .map(|r| {
                        (
                            r.clone(),
                            format!(
                                "#{} {} · {}",
                                fget(r, "display_id", "None"),
                                fget(r, "name", ""),
                                name_of(r.get("category_id"))
                            ),
                        )
                    })
                    .collect();
                self.push(
                    Modal::choices("Service catalog — search items", choices),
                    Cont::CatalogItem,
                );
            }
            (Cont::CatalogItem, R::Choice(row)) => self.launch_create("sr", Some(row)),
            (Cont::Reviewed(step), result) => self.resume_reviewed(step, result),
            (Cont::Assets(step), result) => self.resume_assets(step, result),
            _ => {}
        }
    }
}
