//! Cookie helpers: parsing incoming cookies and building `Set-Cookie`.
//!
//! Covers the Express API surface of `res.cookie()` and
//! `res.clearCookie()`, plus the parsing that the `cookie-parser`
//! middleware does in Express. Here both sides are built in:
//!
//! ```ignore
//! res.cookie("sid", "abc", &CookieOptions::default());
//! req.cookies().get("sid");
//! ```
//!
//! Values on the way out are percent-encoded the way Express's default
//! `encodeURIComponent` encoder does. Values on the way in are
//! percent-decoded; an escape that does not decode is kept raw rather
//! than failing the request.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use httpdate::fmt_http_date;

/// The `SameSite` attribute.
///
/// Express accepts the string form plus `true` for strict; the enum
/// covers both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

/// The `Priority` attribute.
///
/// Express drops `medium` because it is the browser default, so a
/// `Priority::Medium` here is omitted from the header as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    High,
    Medium,
    Low,
}

/// Options for `res.cookie()`, mirroring Express's options object.
#[derive(Debug, Clone, Default)]
pub struct CookieOptions {
    /// Lifetime in seconds.
    ///
    /// Express accepts milliseconds; seconds need no float clamping.
    /// Setting this also derives `Expires` from the current time, the
    /// way Express does, so both attributes are sent.
    pub max_age_secs: Option<u64>,
    /// Absolute expiry time. Express accepts a `Date`.
    ///
    /// Ignored when `max_age_secs` is set, matching Express where
    /// `maxAge` overwrites `expires`.
    pub expires: Option<SystemTime>,
    /// Path scope. Defaults to `/` when unset, as Express does.
    pub path: Option<String>,
    /// Domain scope.
    pub domain: Option<String>,
    /// Send the cookie only over TLS. Defaults to false.
    pub secure: bool,
    /// Hide the cookie from client-side JavaScript. Defaults to false.
    pub http_only: bool,
    /// Cross-site policy. Defaults to unset.
    pub same_site: Option<SameSite>,
    /// Delivery priority. `Medium` is omitted, as Express drops the
    /// browser default.
    pub priority: Option<Priority>,
    /// `Partitioned` attribute. Requires `secure`; when `secure` is
    /// false the attribute is dropped instead of throwing, which
    /// Express does.
    pub partitioned: bool,
}

/// Parse a `Cookie` header into name/value pairs.
///
/// Multiple values for the same name keep the first occurrence. Values
/// are percent-decoded; surrounding quotes are stripped first. Pairs
/// without an `=` or with an empty name are skipped.
pub fn parse(header: &str) -> HashMap<String, String> {
    parse_with(header, true)
}

/// Parse a `Cookie` header without percent-decoding values.
///
/// Signature verification needs the raw bytes: the HMAC covers the
/// still-encoded value, so decoding must happen after it checks out.
pub fn parse_undecoded(header: &str) -> HashMap<String, String> {
    parse_with(header, false)
}

fn parse_with(header: &str, decode: bool) -> HashMap<String, String> {
    let mut map = HashMap::new();

    for pair in header.split(';') {
        let pair = pair.trim();
        let Some(eq) = pair.find('=') else {
            continue;
        };

        let name = pair[..eq].trim();
        if name.is_empty() {
            continue;
        }

        let value = pair[eq + 1..].trim();
        let value = value
            .strip_prefix('"')
            .map(|s| s.strip_suffix('"').unwrap_or(s))
            .unwrap_or(value);
        let value = if decode {
            decode_component(value)
        } else {
            value.to_string()
        };

        map.entry(name.to_string()).or_insert(value);
    }

    map
}

/// Sign a value the way the `cookie-signature` package does:
/// `base64(HMAC-SHA256(secret, value))` with padding stripped.
pub fn sign(value: &str, secret: &str) -> String {
    use hmac::Mac;

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(value.as_bytes());
    base64_nopad(&mac.finalize().into_bytes())
}

/// Verify a signed value, returning the unsigned part when the
/// signature checks out.
///
/// The split happens at the **last** dot, so values containing dots
/// survive. The comparison runs in constant time.
pub fn unsign(signed: &str, secret: &str) -> Option<String> {
    let dot = signed.rfind('.')?;
    let (value, signature) = (&signed[..dot], &signed[dot + 1..]);
    if constant_time_eq(&sign(value, secret), signature) {
        Some(value.to_string())
    } else {
        None
    }
}

/// Compare two strings without short-circuiting on the first
/// difference, as `timingSafeEqual` does.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Standard base64 with the `=` padding removed, as cookie-signature
/// emits.
fn base64_nopad(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);

    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }

    out.trim_end_matches('=').to_string()
}

/// Percent-encode a value the way `encodeURIComponent` does.
///
/// Letters, digits, and `-_.!~*'()` survive; everything else,
/// including the reserved URI characters, becomes `%XX`.
pub fn encode_component(value: &str) -> String {
    const KEPT: &str = "-_.!~*'()";
    let mut out = String::with_capacity(value.len());

    for byte in value.bytes() {
        let c = byte as char;
        if c.is_ascii_alphanumeric() || KEPT.contains(c) {
            out.push(c);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", byte));
        }
    }

    out
}

/// Percent-decode a value, decoding the bytes as UTF-8.
///
/// A `%` not followed by two hex digits, or a byte sequence that is not
/// valid UTF-8, is kept rather than failing the request. Unlike the
/// query parser, `+` is not treated as a space.
pub fn decode_component(value: &str) -> String {
    let input = value.as_bytes();
    let mut bytes = Vec::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        let b = input[i];
        if b == b'%' {
            if let (Some(&h), Some(&l)) = (input.get(i + 1), input.get(i + 2)) {
                if let Some(byte) = hex_pair(h, l) {
                    bytes.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        bytes.push(b);
        i += 1;
    }

    String::from_utf8_lossy(&bytes).to_string()
}

/// Turn two hex digits into a byte, or `None` when either is not hex.
fn hex_pair(h: u8, l: u8) -> Option<u8> {
    let h = (h as char).to_digit(16)?;
    let l = (l as char).to_digit(16)?;
    Some((h * 16 + l) as u8)
}

/// Build a `Set-Cookie` field value.
///
/// The value must already be encoded by the caller. Returns `None` when
/// the name contains characters cookies cannot carry, in place of the
/// `TypeError` Express throws.
pub fn serialize(name: &str, value: &str, options: &CookieOptions) -> Option<String> {
    // Printable ASCII only, the way the `cookie` package validates
    // names; anything with a control character is refused here where
    // Express throws a `TypeError`.
    if name.bytes().any(|b| (b < 0x20 && b != 0x09) || b == 0x7f) {
        return None;
    }

    // maxAge wins over an explicit expires, as it does in Express.
    let expires = match options.max_age_secs {
        Some(secs) => Some(SystemTime::now() + Duration::from_secs(secs)),
        None => options.expires,
    };

    let mut out = format!("{}={}", name, value);

    if let Some(domain) = &options.domain {
        out.push_str("; Domain=");
        out.push_str(domain);
    }

    out.push_str("; Path=");
    out.push_str(options.path.as_deref().unwrap_or("/"));

    if let Some(expires) = expires {
        out.push_str("; Expires=");
        out.push_str(&fmt_http_date(expires));
    }

    if let Some(secs) = options.max_age_secs {
        out.push_str("; Max-Age=");
        out.push_str(&secs.to_string());
    }

    if options.http_only {
        out.push_str("; HttpOnly");
    }

    if options.secure {
        out.push_str("; Secure");
    }

    // Medium is the browser default and is dropped, as Express does.
    if let Some(Priority::High | Priority::Low) = options.priority {
        let label = match options.priority {
            Some(Priority::High) => "High",
            Some(Priority::Low) => "Low",
            _ => unreachable!("only High and Low reach here"),
        };
        out.push_str("; Priority=");
        out.push_str(label);
    }

    if let Some(same_site) = options.same_site {
        out.push_str("; SameSite=");
        out.push_str(match same_site {
            SameSite::Strict => "Strict",
            SameSite::Lax => "Lax",
            SameSite::None => "None",
        });
    }

    if options.partitioned && options.secure {
        out.push_str("; Partitioned");
    }

    Some(out)
}

/// Options for `res.clear_cookie()`.
///
/// Forces the epoch expiry Express uses; a caller-provided `max_age`
/// is dropped because a future lifetime cannot clear anything.
pub(crate) fn clear_options(options: &CookieOptions) -> CookieOptions {
    CookieOptions {
        max_age_secs: None,
        expires: Some(UNIX_EPOCH),
        path: options.path.clone(),
        domain: options.domain.clone(),
        secure: options.secure,
        http_only: options.http_only,
        same_site: options.same_site,
        priority: None,
        partitioned: options.partitioned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_name_value_pairs() {
        let cookies = parse("sid=abc123; theme=dark");
        assert_eq!(cookies.get("sid").map(String::as_str), Some("abc123"));
        assert_eq!(cookies.get("theme").map(String::as_str), Some("dark"));
    }

    #[test]
    fn parse_percent_decodes_values() {
        let cookies = parse("a=hello%20world; b=%C3%A9");
        assert_eq!(cookies.get("a").map(String::as_str), Some("hello world"));
        assert_eq!(cookies.get("b").map(String::as_str), Some("é"));
    }

    #[test]
    fn parse_leaves_broken_escapes_raw() {
        assert_eq!(parse("a=%zz").get("a").map(String::as_str), Some("%zz"));
        assert_eq!(parse("a=%2").get("a").map(String::as_str), Some("%2"));
    }

    #[test]
    fn parse_strips_surrounding_quotes() {
        assert_eq!(
            parse("a=\"quoted value\"").get("a").map(String::as_str),
            Some("quoted value")
        );
    }

    #[test]
    fn parse_keeps_the_first_duplicate() {
        let cookies = parse("a=1; a=2");
        assert_eq!(cookies.get("a").map(String::as_str), Some("1"));
    }

    #[test]
    fn parse_skips_empty_and_unnamed_pairs() {
        let cookies = parse("; flag; =oops; a=1; b=");
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies.get("a").map(String::as_str), Some("1"));
        assert_eq!(cookies.get("b").map(String::as_str), Some(""));
    }

    #[test]
    fn parse_does_not_treat_plus_as_space() {
        assert_eq!(parse("a=b+c").get("a").map(String::as_str), Some("b+c"));
    }

    #[test]
    fn encode_component_keeps_the_unreserved_set() {
        assert_eq!(encode_component("aZ0-_.!~*'()"), "aZ0-_.!~*'()");
    }

    #[test]
    fn encode_component_escapes_space_and_reserved_characters() {
        assert_eq!(encode_component("a b&c=d/e?f"), "a%20b%26c%3Dd%2Fe%3Ff");
    }

    #[test]
    fn encode_component_escapes_multibyte_utf8() {
        assert_eq!(encode_component("é"), "%C3%A9");
    }

    #[test]
    fn decode_component_round_trips() {
        let encoded = encode_component("héllo wörld & =?");
        assert_eq!(decode_component(&encoded), "héllo wörld & =?");
    }

    #[test]
    fn serialize_defaults_to_path_root() {
        let field = serialize("sid", "abc", &CookieOptions::default()).unwrap();
        assert_eq!(field, "sid=abc; Path=/");
    }

    #[test]
    fn serialize_emits_attributes_in_cookie_package_order() {
        let options = CookieOptions {
            max_age_secs: Some(3600),
            path: Some("/app".to_string()),
            domain: Some("example.com".to_string()),
            secure: true,
            http_only: true,
            same_site: Some(SameSite::Strict),
            priority: Some(Priority::High),
            ..Default::default()
        };
        let field = serialize("sid", "abc", &options).unwrap();

        let parts: Vec<&str> = field.split("; ").collect();
        let names: Vec<&str> = parts.iter().map(|p| p.split('=').next().unwrap()).collect();
        assert_eq!(
            names,
            vec![
                "sid", "Domain", "Path", "Expires", "Max-Age", "HttpOnly", "Secure", "Priority",
                "SameSite",
            ]
        );
        assert!(parts[3].starts_with("Expires="));
    }

    #[test]
    fn serialize_derives_expires_from_max_age() {
        let options = CookieOptions {
            max_age_secs: Some(60),
            ..Default::default()
        };
        let field = serialize("sid", "abc", &options).unwrap();
        assert!(field.contains("; Max-Age=60"), "got: {}", field);
        assert!(field.contains("; Expires="), "got: {}", field);
    }

    #[test]
    fn serialize_drops_medium_priority() {
        let options = CookieOptions {
            priority: Some(Priority::Medium),
            ..Default::default()
        };
        let field = serialize("sid", "abc", &options).unwrap();
        assert!(!field.contains("Priority"), "got: {}", field);
    }

    #[test]
    fn serialize_drops_partitioned_without_secure() {
        let options = CookieOptions {
            partitioned: true,
            ..Default::default()
        };
        let field = serialize("sid", "abc", &options).unwrap();
        assert!(!field.contains("Partitioned"), "got: {}", field);

        let options = CookieOptions {
            partitioned: true,
            secure: true,
            ..Default::default()
        };
        let field = serialize("sid", "abc", &options).unwrap();
        assert!(field.contains("; Partitioned"), "got: {}", field);
    }

    #[test]
    fn serialize_rejects_control_characters_in_names() {
        // Printable ASCII passes, '=' included, as the cookie package
        // allows; only control characters and DEL are refused.
        assert!(serialize("a=b", "v", &CookieOptions::default()).is_some());
        assert!(serialize("a b", "v", &CookieOptions::default()).is_some());
        assert!(serialize("a\u{1}b", "v", &CookieOptions::default()).is_none());
        assert!(serialize("a\u{7f}", "v", &CookieOptions::default()).is_none());
    }

    #[test]
    fn serialize_handles_empty_value() {
        let field = serialize("sid", "", &CookieOptions::default()).unwrap();
        assert_eq!(field, "sid=; Path=/");
    }

    #[test]
    fn sign_matches_the_cookie_signature_format() {
        // Pinned against Node's crypto:
        //   base64(HMAC-SHA256('secret', 'value')) with padding stripped.
        assert_eq!(
            sign("value", "secret"),
            "UOA+vmW+mLuL8RuiyJLVTAeayisNOwFidpxtdXolQ08"
        );
    }

    #[test]
    fn unsign_round_trips_and_rejects_tampering() {
        let signed = format!("{}.{}", "a%20b", sign("a%20b", "topsecret"));
        assert_eq!(unsign(&signed, "topsecret").as_deref(), Some("a%20b"));

        let tampered = signed.replace("a%20b", "a%20c");
        assert_eq!(unsign(&tampered, "topsecret"), None);
        assert_eq!(unsign(&signed, "wrong"), None);
        assert_eq!(unsign("no-dot", "secret"), None);
    }

    #[test]
    fn unsign_splits_at_the_last_dot() {
        let signed = format!("{}.{}", "a.b", sign("a.b", "secret"));
        assert_eq!(unsign(&signed, "secret").as_deref(), Some("a.b"));
    }

    #[test]
    fn base64_nopad_handles_partial_chunks() {
        assert_eq!(base64_nopad(b"f"), "Zg");
        assert_eq!(base64_nopad(b"fo"), "Zm8");
        assert_eq!(base64_nopad(b"foo"), "Zm9v");
    }

    #[test]
    fn parse_undecoded_keeps_percent_escapes() {
        let cookies = parse_undecoded("a=hello%20world");
        assert_eq!(cookies.get("a").map(String::as_str), Some("hello%20world"));
    }

    #[test]
    fn clear_options_forces_epoch_expiry_and_drops_max_age() {
        let options = CookieOptions {
            max_age_secs: Some(60),
            path: Some("/app".to_string()),
            http_only: true,
            priority: Some(Priority::High),
            ..Default::default()
        };
        let cleared = clear_options(&options);
        let field = serialize("sid", "", &cleared).unwrap();

        assert_eq!(
            field,
            "sid=; Path=/app; Expires=Thu, 01 Jan 1970 \
                           00:00:00 GMT; HttpOnly"
        );
    }
}
