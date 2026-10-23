//! Versioned, stdin-only bridge for native shell editors. No execution or history access.
use crate::{
    backend::Backend,
    context::{Context, unsafe_char},
    risk::{RiskPolicy, RiskReview, review},
    shell::ShellContext,
};
use serde::{Deserialize, Serialize};
use std::io::Read;

pub const MAX_REQUEST_BYTES: u64 = 24 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub protocol: u32,
    pub request_id: String,
    pub shell: ShellContext,
    pub cwd: String,
    pub buffer: String,
    pub cursor: usize,
}
#[derive(Serialize)]
pub struct Response {
    pub protocol: u32,
    pub request_id: String,
    pub buffer: String,
    pub cursor: usize,
    pub insertion: String,
    pub result: String,
    pub result_cursor: usize,
    pub risk_review: RiskReview,
    pub additional_acceptance_required: bool,
}

pub fn read_request(reader: impl Read) -> Result<Request, &'static str> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read adapter request")?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("adapter request exceeds 24 KiB limit");
    }
    // Parser errors can contain untrusted/private field names. Never expose them.
    serde_json::from_slice(&bytes).map_err(|_| "invalid adapter request JSON")
}

pub fn complete(
    request: Request,
    backend: &dyn Backend,
) -> Result<Response, Box<dyn std::error::Error>> {
    if request.protocol != 1 {
        return Err("unsupported adapter protocol".into());
    }
    if request.request_id.is_empty()
        || request.request_id.len() > 128
        || !request
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err("invalid adapter request identity".into());
    }
    if request.cwd.is_empty() || request.cwd.len() > 4096 || request.cwd.chars().any(unsafe_char) {
        return Err("invalid adapter cwd metadata".into());
    }
    // cwd is validated but deliberately never added to Context, read, or sent to a provider.
    let context = Context::new(&request.buffer, request.cursor)?.with_shell(request.shell)?;
    let insertion = if context.is_empty() {
        String::new()
    } else {
        backend.complete(&context)?
    };
    let result = context.insert(&insertion)?;
    let risk_review = review(context.shell(), &result);
    let additional_acceptance_required =
        !insertion.is_empty() && risk_review.requires_confirmation(RiskPolicy::Confirm);
    Ok(Response {
        protocol: 1,
        request_id: request.request_id,
        buffer: request.buffer,
        cursor: request.cursor,
        result_cursor: request.cursor + insertion.len(),
        insertion,
        result,
        risk_review,
        additional_acceptance_required,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::{BackendError, MockBackend},
        shell::ShellKind,
    };
    fn request(buffer: &str, cursor: usize) -> Request {
        Request {
            protocol: 1,
            request_id: "test-1".into(),
            shell: ShellContext::new(ShellKind::Bash),
            cwd: "/synthetic-private-path".into(),
            buffer: buffer.into(),
            cursor,
        }
    }
    #[test]
    fn fim_and_unicode() {
        let r = complete(request("git  --short", 4), &MockBackend).unwrap();
        assert_eq!(r.result, "git status --short");
        assert_eq!(r.result_cursor, 10);
        let r = complete(request("echo 世", 8), &MockBackend).unwrap();
        assert_eq!(r.result, "echo 世界");
        assert_eq!(r.result_cursor, 11);
    }
    struct Never;
    impl Backend for Never {
        fn complete(&self, _: &Context) -> Result<String, BackendError> {
            panic!("blank called backend")
        }
    }
    #[test]
    fn blank_skips_backend() {
        assert!(
            complete(request("  ", 2), &Never)
                .unwrap()
                .insertion
                .is_empty()
        );
    }
    #[test]
    fn validates_protocol_identity_cursor_cwd() {
        let mut r = request("git", 3);
        r.protocol = 2;
        assert!(complete(r, &Never).is_err());
        let mut r = request("git", 3);
        r.request_id = "a\nb".into();
        assert!(complete(r, &Never).is_err());
        let mut r = request("git", 3);
        r.cwd = "a\nb".into();
        assert!(complete(r, &Never).is_err());
        assert!(complete(request("世", 1), &Never).is_err());
        assert!(read_request(&b"{\"PRIVATE\":1}"[..]).is_err());
        assert!(read_request(&vec![b' '; MAX_REQUEST_BYTES as usize + 1][..]).is_err());
    }
    struct Dangerous;
    impl Backend for Dangerous {
        fn complete(&self, c: &Context) -> Result<String, BackendError> {
            assert!(
                !serde_json::to_string(c)
                    .unwrap()
                    .contains("synthetic-private-path")
            );
            Ok("rm -rf /tmp/synthetic".into())
        }
    }
    #[test]
    fn risk_cannot_be_silenced_and_cwd_stays_local() {
        assert!(
            complete(request("echo x; ", 8), &Dangerous)
                .unwrap()
                .additional_acceptance_required
        );
    }
}
