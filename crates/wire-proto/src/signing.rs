//! Canonical byte-string construction for signed directory-server requests.
//!
//! Every write request is signed with the caller's Ed25519 identity key over a
//! domain-separated, field-separated byte string (built here) rather than raw
//! JSON — this sidesteps JSON canonicalization ambiguity (field order, whitespace)
//! entirely, at the cost of each request type spelling out its own field list.
//! The domain tag (e.g. "register-user/v1") prevents a validly-signed payload
//! for one endpoint from being replayed against a different one.

const FIELD_SEP: char = '\u{1f}'; // ASCII unit separator, won't appear in normal field values

pub fn join(domain: &str, parts: &[&str]) -> Vec<u8> {
    let mut s = String::from(domain);
    for p in parts {
        s.push(FIELD_SEP);
        s.push_str(p);
    }
    s.into_bytes()
}
