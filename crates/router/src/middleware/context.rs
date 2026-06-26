//! Request context extraction from JSON body and HTTP headers.

use axum::http::HeaderMap;

use crate::policies::RequestContext;

/// Extract [`RequestContext`] from the raw request body bytes and headers.
///
/// Session affinity routing key is determined in this priority order:
/// 1. Implicit key from stable headers: `authorization`, `x-forwarded-for`,
///    or `cookie` (first non-empty value wins).
/// 2. Explicit fields in the JSON body: `"user"`, then `"session_id"`.
/// 3. Fallback to `"default"`.
///
/// `prefix_key`: first 200 characters of the first message's `"content"`,
/// preferring `system`-role messages, defaulting to `"default"`.
pub(crate) fn extract_context(body: &[u8], headers: &HeaderMap) -> RequestContext {
    let default = RequestContext {
        session_id: "default".to_string(),
        prefix_key: "default".to_string(),
    };

    // Implicit routing key from stable headers (session affinity).
    let implicit_key = headers
        .get("authorization")
        .or_else(|| headers.get("x-forwarded-for"))
        .or_else(|| headers.get("cookie"))
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty());

    let v: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => {
            if let Some(key) = implicit_key {
                return RequestContext {
                    session_id: key.to_string(),
                    prefix_key: "default".to_string(),
                };
            }
            return default;
        }
    };

    // Use header-based implicit key for session_id if available,
    // otherwise fall back to body fields.
    let session_id = if let Some(key) = implicit_key {
        key.to_string()
    } else {
        v.get("user")
            .or_else(|| v.get("session_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "default".to_string())
    };

    let prefix_key = v
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|msgs| {
            // Prefer the first system-role message.
            msgs.iter()
                .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
                .or_else(|| msgs.first())
        })
        .and_then(|msg| msg.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| {
            // Take up to 200 chars (clamp at a char boundary).
            s.char_indices()
                .take(200)
                .last()
                .map(|(idx, c)| &s[..idx + c.len_utf8()])
                .unwrap_or(s)
                .to_string()
        })
        .unwrap_or_else(|| "default".to_string());

    RequestContext {
        session_id,
        prefix_key,
    }
}
