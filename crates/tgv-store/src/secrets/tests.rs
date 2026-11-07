use super::*;

/// Synthetic tokens, assembled at run time so no complete token-shaped
/// literal appears in the source (and trips secret scanning on push).
fn fake(prefix: &str, body: &str, repeat: usize) -> String {
    format!("{prefix}{}", body.repeat(repeat))
}

fn kinds(scanner: &SecretScanner, text: &str) -> Vec<SecretKind> {
    scanner
        .find(text)
        .into_iter()
        .map(|(_, kind)| kind)
        .collect()
}

#[test]
fn well_known_formats_are_found_by_kind() {
    let scanner = SecretScanner::new().unwrap();
    let cases = [
        (fake("sk-ant-", "api03xY9z", 5), SecretKind::AnthropicKey),
        (fake("sk-proj-", "Ab1Cd2Ef3", 4), SecretKind::OpenaiKey),
        (fake("sk-", "Zz9Yy8Xx7", 4), SecretKind::OpenaiKey),
        (
            fake("gh", "p_", 1) + &"A1b2C3d4E5f6".repeat(3),
            SecretKind::GithubToken,
        ),
        (
            fake("github_", "pat_", 1) + &"Q1w2E3r4T5y6".repeat(3),
            SecretKind::GithubToken,
        ),
        (
            fake("AK", "IA", 1) + "ABCDEFGHIJKLMNOP",
            SecretKind::AwsAccessKey,
        ),
        (
            fake("AI", "za", 1) + &"B".repeat(35),
            SecretKind::GoogleApiKey,
        ),
        (
            fake("xo", "xb-", 1) + "1234567890-abcdef",
            SecretKind::SlackToken,
        ),
        (
            fake("sk_", "live_", 1) + &"a1B2c3D4e5".repeat(3),
            SecretKind::StripeLiveKey,
        ),
        (
            format!(
                "-----BEGIN {}PRIVATE KEY-----\nMIIE\n-----END {}PRIVATE KEY-----",
                "RSA ", "RSA "
            ),
            SecretKind::PrivateKey,
        ),
        (
            format!(
                "{}.{}.{}",
                fake("eyJ", "hbGciOi", 2),
                fake("eyJ", "zdWIiOi", 2),
                "c2lnbmF0dXJl"
            ),
            SecretKind::Jwt,
        ),
    ];
    for (secret, kind) in &cases {
        let text = format!("export TOKEN={secret} # in a shell");
        assert_eq!(kinds(&scanner, &text), [*kind], "{secret}");
    }
}

#[test]
fn ordinary_text_is_not_a_secret() {
    let scanner = SecretScanner::new().unwrap();
    for text in [
        "sk-short",
        "the task-ant-colony example",
        "commit 3f9821cf19f147a1a70e2a09c294db3c and sha256:abcdef0123456789",
        "AKIA is a prefix; AKIAABC is too short",
        "-----BEGIN PUBLIC KEY-----",
        "ghp_tooShort",
        "eyJhbGciOiJIUzI1NiJ9 alone is a header, not a token",
    ] {
        assert!(kinds(&scanner, text).is_empty(), "{text}");
    }
    // A test-mode Stripe key is not a live one. Assembled at run time, like
    // every token here.
    let test_mode = fake("rk_", "test_", 1) + &"1234567890".repeat(3);
    assert!(kinds(&scanner, &test_mode).is_empty());
}

#[test]
fn redaction_replaces_secrets_and_keeps_only_a_hint_and_fingerprint() {
    let scanner = SecretScanner::new().unwrap();
    let key = fake("sk-ant-", "api03xY9z", 5) + "a1b2";
    let other = fake("sk-ant-", "api03qQ8w", 5) + "c3d4";
    let text = format!("first {key}, again {key}, then {other}.");
    let (redacted, found) = scanner.redact(&text);
    assert_eq!(
        redacted,
        "first «redacted: anthropic_key», again «redacted: anthropic_key», then «redacted: anthropic_key»."
    );
    assert!(!redacted.contains("api03"));
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].hint, "sk-ant-…a1b2");
    assert_eq!(found[2].hint, "sk-ant-…c3d4");
    // The same secret has the same fingerprint; a different one does not.
    assert_eq!(found[0].fingerprint, found[1].fingerprint);
    assert_ne!(found[0].fingerprint, found[2].fingerprint);
    assert_eq!(found[0].fingerprint.len(), 16);
    assert!(!key.contains(&found[0].fingerprint));
    // Nothing found: the text is unchanged.
    assert_eq!(scanner.redact("plain text").0, "plain text");
}

#[test]
fn overlapping_formats_keep_the_earlier_longer_match() {
    let scanner = SecretScanner::new().unwrap();
    // A private key block containing something token-shaped is one secret.
    let block = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
        fake("sk-", "Zz9Yy8Xx7", 4)
    );
    assert_eq!(kinds(&scanner, &block), [SecretKind::PrivateKey]);
    // A header without its end (a cut-off excerpt) is still reported.
    assert_eq!(
        kinds(&scanner, "-----BEGIN EC PRIVATE KEY-----\nMHcCAQEE"),
        [SecretKind::PrivateKey]
    );
    let (redacted, found) = scanner.redact(&block);
    assert_eq!(redacted, "«redacted: private_key»");
    assert_eq!(found[0].hint, "-----BEGIN … PRIVATE KEY-----");
}
