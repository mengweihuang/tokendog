//! Request context extraction from JSON body.

use crate::policies::RequestContext;

/// Extract [`RequestContext`] from the raw request body bytes.
///
/// Parses the JSON body to pull out:
/// - `session_id`: from `"user"` field, falling back to `"session_id"`,
///   then to `"default"`.
/// - `prefix_key`: first 200 characters of the first message's `"content"`,
///   preferring `system`-role messages, defaulting to `"default"`.
pub(crate) fn extract_context(body: &[u8]) -> RequestContext {
    let default = RequestContext {
        session_id: "default".to_string(),
        prefix_key: "default".to_string(),
    };

    let v: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return default,
    };

    let session_id = v
        .get("user")
        .or_else(|| v.get("session_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "default".to_string());

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
