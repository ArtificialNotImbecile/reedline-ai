//! Strict bounded SSE chat-completion decoder. Partial text is display-only.
use crate::{backend::BackendError, context::validate_insertion};
use serde_json::Value;

#[derive(Default)]
pub(crate) struct SseCompletion {
    line: Vec<u8>,
    data: Vec<u8>,
    text: String,
    stopped: bool,
    done: bool,
    events: usize,
}
impl SseCompletion {
    pub fn is_done(&self) -> bool {
        self.done
    }
    pub fn push(
        &mut self,
        chunk: &[u8],
        preview: &mut dyn FnMut(&str),
    ) -> Result<(), BackendError> {
        for &byte in chunk {
            if self.done {
                break;
            }
            if byte != b'\n' {
                self.line.push(byte);
                if self.line.len() > 16 * 1024 {
                    return Err(BackendError::BodyTooLarge);
                }
                continue;
            }
            if self.line.last() == Some(&b'\r') {
                self.line.pop();
            }
            let line = std::mem::take(&mut self.line);
            if line.is_empty() {
                if !self.data.is_empty() {
                    self.event(preview)?;
                }
            } else if let Some(value) = line.strip_prefix(b"data:") {
                let value = value.strip_prefix(b" ").unwrap_or(value);
                if !self.data.is_empty() {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(value);
                if self.data.len() > 16 * 1024 {
                    return Err(BackendError::BodyTooLarge);
                }
            } // Ignore comments and SSE transport metadata (event, id, retry).
        }
        Ok(())
    }
    fn event(&mut self, preview: &mut dyn FnMut(&str)) -> Result<(), BackendError> {
        self.events += 1;
        if self.events > 4096 {
            return Err(BackendError::BodyTooLarge);
        }
        let bytes = std::mem::take(&mut self.data);
        if bytes == b"[DONE]" {
            if !self.stopped {
                return Err(BackendError::InvalidResponse);
            }
            self.done = true;
            return Ok(());
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| BackendError::InvalidResponse)?;
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .ok_or(BackendError::InvalidResponse)?;
        if choices.is_empty() && value.get("usage").is_some() {
            return Ok(());
        }
        if choices.len() != 1 {
            return Err(BackendError::InvalidResponse);
        }
        let choice = &choices[0];
        if choice.get("index").is_some_and(|i| i.as_u64() != Some(0)) {
            return Err(BackendError::InvalidResponse);
        }
        let delta = choice
            .get("delta")
            .and_then(Value::as_object)
            .ok_or(BackendError::InvalidResponse)?;
        if delta
            .get("role")
            .is_some_and(|v| !v.is_null() && v != "assistant")
        {
            return Err(BackendError::InvalidResponse);
        }
        if ["tool_calls", "function_call", "refusal"]
            .iter()
            .any(|key| delta.get(*key).is_some_and(|v| !v.is_null()))
        {
            return Err(BackendError::InvalidResponse);
        }
        if let Some(content) = delta.get("content").filter(|v| !v.is_null()) {
            let content = content.as_str().ok_or(BackendError::InvalidResponse)?;
            if self.stopped && !content.is_empty() {
                return Err(BackendError::InvalidResponse);
            }
            self.text.push_str(content);
            validate_insertion(&self.text).map_err(BackendError::UnsafeOutput)?;
            if !content.is_empty() {
                preview(&self.text);
            }
        }
        if let Some(reason) = choice.get("finish_reason").filter(|v| !v.is_null()) {
            if reason != "stop" || self.stopped {
                return Err(BackendError::InvalidResponse);
            }
            self.stopped = true;
        }
        Ok(())
    }
    pub fn finish(self) -> Result<String, BackendError> {
        if !self.done || !self.stopped {
            return Err(BackendError::InvalidResponse);
        }
        validate_insertion(&self.text).map_err(BackendError::UnsafeOutput)?;
        Ok(self.text)
    }
}
