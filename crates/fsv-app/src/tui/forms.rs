//! The TUI's `N` / `e` forms (`app._build_form`, `_form_session`): the
//! fsv-formui session for the request, embedded as a [`FormScreen`].
//!
//! A [`FormSession`] is not `Send`, so each form lives on its own thread
//! (built there, as Python builds it on a worker) and the TUI drives it
//! through a [`RemoteForm`] proxy; the submitted plan is sent from that
//! thread too.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde_json::Value;

use fsv_core::{Error, Result};
use fsv_formui::kit::theme::Theme;
use fsv_formui::{Action, FormMode, FormScreen, FormSession, Plan};
use fsv_tui::backend::{
    BResult, Buffer, Failure, FormExit, FormRequest, FormSend, FormView, KeyEvent, Position, Rect,
};

use crate::form::{
    ChangeForm, FormDeps, SendOutcome, Submitted, change_session, sr_session, ticket_session,
};

/// Makes the [`FormDeps`] a form talks to, on the form's thread.
pub type DepsFactory = Arc<dyn Fn() -> Result<FormDeps> + Send + Sync>;

/// `_send_form`'s `session.send(result.plan)`; a partial failure carries
/// the attributes `failure_detail` lists.
pub fn send_submitted(s: Submitted) -> BResult<Value> {
    match s.send_checked(false) {
        Ok(SendOutcome::Sent(v)) => Ok(v),
        Ok(SendOutcome::Changed(c)) => Err(Failure::from(c.refusal())),
        Err(e) => Err(Failure {
            outcome: e.outcome(),
            error: Box::new(e.into_error()),
        }),
    }
}

/// `_build_form`'s session: `ticket_session` (incident), `sr_session` for
/// the picked catalog item (sr), else `_form_session` (a new change, or an
/// edit of the record with `target = display_id or id`).
pub fn session(deps: &FormDeps, req: &FormRequest) -> Result<FormSession> {
    match req.mode.as_str() {
        "incident" => ticket_session(deps, None, None, false),
        "sr" => {
            let item_ref = req
                .item
                .as_ref()
                .and_then(|i| i.get("display_id"))
                .ok_or_else(|| Error::other("'display_id'"))?;
            sr_session(deps, &fsv_py::py_str(item_ref), None, None, false)
        }
        "new" => change_session(deps, ChangeForm::new(FormMode::New)),
        _ => {
            let record = req.record.clone().map(|mut r| {
                if let Some(m) = r.as_object_mut() {
                    m.shift_remove("_resource");
                }
                r
            });
            let target = record.as_ref().and_then(|r| {
                r.get("display_id")
                    .filter(|v| fsv_py::truthy(v))
                    .or_else(|| r.get("id"))
                    .cloned()
            });
            change_session(
                deps,
                ChangeForm {
                    target,
                    record,
                    ..ChangeForm::new(FormMode::Edit)
                },
            )
        }
    }
}

enum Cmd {
    Tick(Duration),
    Key(KeyEvent),
    Render(Rect, Buffer, Sender<(Buffer, Option<Position>)>),
    Done(Sender<bool>),
    Clipboard(Sender<Option<String>>),
    Finish(Sender<Finished>),
    Send(Sender<BResult<Value>>),
}

enum Finished {
    Cancelled,
    Draft(String),
    Submit,
}

/// Open the form for `req` on its own thread; `Err` when the session
/// cannot be built.
pub fn open(deps: DepsFactory, req: FormRequest) -> Result<RemoteForm> {
    let (tx, rx) = mpsc::channel::<Cmd>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
    std::thread::spawn(move || {
        let built = deps().and_then(|d| session(&d, &req));
        let session = match built {
            Ok(s) => s,
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };
        let _ = ready_tx.send(Ok(()));
        serve(FormScreen::new(session, Theme::default()), &rx);
    });
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(RemoteForm { tx }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(Error::other("the form thread stopped")),
    }
}

fn serve(screen: FormScreen, rx: &Receiver<Cmd>) {
    let mut screen = Some(screen);
    let mut submitted: Option<(FormSession, Plan)> = None;
    for cmd in rx {
        match (cmd, screen.as_mut()) {
            (Cmd::Tick(now), Some(s)) => s.tick(now),
            (Cmd::Key(k), Some(s)) => s.handle_key(k),
            (Cmd::Render(area, mut buf, reply), Some(s)) => {
                let pos = s.render(area, &mut buf);
                let _ = reply.send((buf, pos));
            }
            (Cmd::Done(reply), s) => {
                let _ = reply.send(s.is_none_or(|s| s.done().is_some()));
            }
            (Cmd::Clipboard(reply), s) => {
                let _ = reply.send(s.and_then(FormScreen::take_clipboard));
            }
            (Cmd::Finish(reply), _) => {
                let Some(s) = screen.take() else {
                    let _ = reply.send(Finished::Cancelled);
                    continue;
                };
                let result = s.into_result();
                let finished = match (result.action, result.plan) {
                    (Action::SaveDraft, _) => Finished::Draft(
                        result
                            .saved_path
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    ),
                    (Action::Submit, Some(plan)) => {
                        submitted = Some((result.session, plan));
                        Finished::Submit
                    }
                    _ => Finished::Cancelled,
                };
                let _ = reply.send(finished);
            }
            (Cmd::Send(reply), _) => {
                let out = match submitted.take() {
                    Some((session, plan)) => send_submitted(Submitted { plan, session }),
                    None => Err(Failure::from(Error::other("nothing to send"))),
                };
                let _ = reply.send(out);
                return;
            }
            _ => {}
        }
    }
}

/// The TUI's handle on a form running on its own thread.
pub struct RemoteForm {
    tx: Sender<Cmd>,
}

impl RemoteForm {
    fn ask<T>(&self, cmd: impl FnOnce(Sender<T>) -> Cmd) -> Option<T> {
        let (reply, answer) = mpsc::channel();
        self.tx.send(cmd(reply)).ok()?;
        answer.recv().ok()
    }
}

impl FormView for RemoteForm {
    fn tick(&mut self, now: Duration) {
        let _ = self.tx.send(Cmd::Tick(now));
    }

    fn handle_key(&mut self, key: &KeyEvent) {
        let _ = self.tx.send(Cmd::Key(*key));
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) -> Option<Position> {
        let (drawn, pos) = self.ask(|r| Cmd::Render(area, buf.clone(), r))?;
        *buf = drawn;
        pos
    }

    fn done(&self) -> bool {
        self.ask(Cmd::Done).unwrap_or(true)
    }

    fn take_clipboard(&mut self) -> Option<String> {
        self.ask(Cmd::Clipboard).flatten()
    }

    fn finish(self: Box<Self>) -> FormExit {
        match self.ask(Cmd::Finish) {
            Some(Finished::Draft(path)) => FormExit::Draft(path),
            Some(Finished::Submit) => FormExit::Submit(Box::new(RemoteSend { tx: self.tx })),
            _ => FormExit::Cancelled,
        }
    }
}

/// `session.send(result.plan)` on the form's thread.
pub struct RemoteSend {
    tx: Sender<Cmd>,
}

impl FormSend for RemoteSend {
    fn send(self: Box<Self>) -> BResult<Value> {
        let (reply, answer) = mpsc::channel();
        self.tx
            .send(Cmd::Send(reply))
            .map_err(|_| Failure::from(Error::other("the form thread stopped")))?;
        answer
            .recv()
            .map_err(|_| Failure::from(Error::other("the form thread stopped")))?
    }
}
