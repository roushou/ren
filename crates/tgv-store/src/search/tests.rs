use super::*;

#[test]
fn terms_become_quoted_phrases_so_input_never_injects_syntax() {
    assert_eq!(
        SearchTerms::parse("cargo  test").unwrap().expression(),
        r#""cargo" AND "test""#
    );
    assert_eq!(
        SearchTerms::parse(r#"run "install vps" now"#)
            .unwrap()
            .expression(),
        r#""run" AND "install vps" AND "now""#
    );
    // FTS5 operators and stray quotes are plain text inside a phrase.
    assert_eq!(
        SearchTerms::parse(r#"a OR b NEAR(c) col:x *"#)
            .unwrap()
            .expression(),
        r#""a" AND "OR" AND "b" AND "NEAR(c)" AND "col:x" AND "*""#
    );
    assert_eq!(
        SearchTerms::parse(r#"unclosed "quote"#)
            .unwrap()
            .expression(),
        r#""unclosed" AND "quote""#
    );
    let phrase = SearchTerms::parse(r#" run "install vps" "#).unwrap();
    assert_eq!(SearchTerms::parse(&phrase.to_string()), Ok(phrase));
    assert_eq!(SearchTerms::parse("   "), Err(SearchError::Terms));
    assert_eq!(SearchTerms::parse(r#""  ""#), Err(SearchError::Terms));
    assert_eq!(
        SearchTerms::parse(&"x".repeat(SearchTerms::MAX_BYTES + 1)),
        Err(SearchError::Terms)
    );
}
