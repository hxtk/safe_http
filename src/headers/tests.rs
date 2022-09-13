#[test]
fn normalize_header_value_with_crlf() {
    let got = super::normalize_header_value("Foo Bar\r\nBaz: Foo");
    assert_eq!(got, "Foo Bar Baz: Foo");
}
