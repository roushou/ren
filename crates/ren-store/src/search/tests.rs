use super::*;

#[test]
fn terms_become_quoted_phrases_so_input_never_injects_syntax() {
    assert_eq!(
        "cargo  test".parse::<SearchTerms>().unwrap().expression(),
        r#""cargo" AND "test""#
    );
    assert_eq!(
        r#"run "install vps" now"#.parse::<SearchTerms>().unwrap().expression(),
        r#""run" AND "install vps" AND "now""#
    );
    // FTS5 operators and stray quotes are plain text inside a phrase.
    assert_eq!(
        r#"a OR b NEAR(c) col:x *"#.parse::<SearchTerms>().unwrap().expression(),
        r#""a" AND "OR" AND "b" AND "NEAR(c)" AND "col:x" AND "*""#
    );
    assert_eq!(
        r#"unclosed "quote"#.parse::<SearchTerms>().unwrap().expression(),
        r#""unclosed" AND "quote""#
    );
    let phrase = r#" run "install vps" "#.parse::<SearchTerms>().unwrap();
    assert_eq!(phrase.to_string().parse::<SearchTerms>(), Ok(phrase));
    assert_eq!("   ".parse::<SearchTerms>(), Err(SearchError::Terms));
    assert_eq!(r#""  ""#.parse::<SearchTerms>(), Err(SearchError::Terms));
    assert_eq!(
        "x".repeat(SearchTerms::MAX_BYTES + 1)
            .parse::<SearchTerms>(),
        Err(SearchError::Terms)
    );
}
