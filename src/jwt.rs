use crate::data::{AuthFile, JwtPayload};
use base64::{
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};

/// Decode the JWT id_token and extract the email from the payload.
pub fn extract_email(auth: &AuthFile) -> Option<String> {
    extract_email_from_token(&auth.tokens.id_token)
}

pub fn extract_email_from_token(token: &str) -> Option<String> {
    decode_token_payload(token)?.email.or_else(|| {
        decode_token_payload(token)?
            .openai_profile
            .and_then(|profile| profile.email)
    })
}

pub fn decode_token_payload(token: &str) -> Option<JwtPayload> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    // Base64url-decode the payload (part[1])
    let payload_b64 = parts[1];

    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64.as_bytes())
        .or_else(|_| {
            let mut padded = payload_b64.to_string();
            while !padded.len().is_multiple_of(4) {
                padded.push('=');
            }
            URL_SAFE.decode(padded.as_bytes())
        })
        .ok()?;

    let payload_str = match std::str::from_utf8(&payload_bytes) {
        Ok(s) => s,
        Err(_) => return None,
    };

    serde_json::from_str(payload_str).ok()
}
