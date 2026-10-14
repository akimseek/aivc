//! Completed Codex turn boundaries, shared by paging and branch validation.
use aivc_core::session::Role;
use aivc_core::text::should_keep_message;
use serde_json::Value;

use super::parser::extract_message;

#[derive(Debug, Default)]
pub(crate) struct TurnTracker {
    active_id: Option<String>,
    last_assistant_line: Option<u64>,
    previous: Option<(Role, String)>,
}

#[derive(Debug)]
pub(crate) struct CompletedTurn {
    pub id: String,
    pub message_line: u64,
}

impl TurnTracker {
    /// Only an explicit completion event makes the final assistant eligible.
    /// The completion ID also identifies a turn when reading starts mid-turn.
    pub fn push(&mut self, value: &Value, line_number: u64) -> Option<CompletedTurn> {
        let kind = value["type"].as_str().unwrap_or_default();
        let payload = &value["payload"];
        let event = payload["type"].as_str().unwrap_or_default();
        if kind == "turn_context" || (kind == "event_msg" && event == "task_started") {
            if let Some(id) = payload["turn_id"].as_str().filter(|id| !id.is_empty()) {
                if self.active_id.as_deref().is_some_and(|active| active != id) {
                    self.last_assistant_line = None;
                }
                self.active_id = Some(id.to_owned());
            }
        }
        if kind == "event_msg" && matches!(event, "turn_aborted" | "task_aborted") {
            self.last_assistant_line = None;
            self.active_id = None;
            return None;
        }
        if kind == "event_msg" && event == "task_complete" {
            let explicit_id = payload["turn_id"].as_str().filter(|id| !id.is_empty());
            if explicit_id
                .is_some_and(|id| self.active_id.as_deref().is_some_and(|active| active != id))
            {
                return None;
            }
            let id = explicit_id.or(self.active_id.as_deref())?.to_owned();
            let message_line = self.last_assistant_line.take()?;
            return Some(CompletedTurn { id, message_line });
        }
        if let Some(message) =
            extract_message(payload, kind).filter(|message| should_keep_message(&message.text))
        {
            // event_msg and response_item can contain the same message. Keep
            // the first raw line, matching the paging fold's deduplication.
            if self
                .previous
                .as_ref()
                .is_some_and(|(role, text)| *role == message.role && text == &message.text)
            {
                return None;
            }
            if message.role == Role::Assistant {
                self.last_assistant_line = Some(line_number);
            } else if message.role == Role::User {
                self.last_assistant_line = None;
            }
            self.previous = Some((message.role, message.text));
        }
        None
    }
}
