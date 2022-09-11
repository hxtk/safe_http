
use std::assert_matches::assert_matches;
use std::convert::TryFrom;

use super::Uri;
use super::HierPart;
use super::Rootless;

#[test]
fn parse_roundtrips() {
    let inputs = [
        "ftp://ftp.is.co.za/rfc/rfc1808.txt",
        "http://www.ietf.org/rfc/rfc2396.txt",
        "ldap://[2001:db8::7]/c=GB?objectClass?one",
        "mailto:John.Doe@example.com",
        "news:comp.infosystems.www.servers.unix",
        "tel:+1-816-555-1212",
        "telnet://192.0.2.16:80/",
        "urn:oasis:names:specification:docbook:dtd:xml:4.1.2",
    ];
    for input in inputs {
        println!("{}", input);
        let x = Uri::try_from(input).unwrap();
        println!("{:#?}", x);
        assert_eq!(input, x.to_string());
    }
}

#[test]
fn parse_mailto() {
    let input = "mailto:John.Doe@example.com";
    let x = Uri::try_from(input).unwrap();
    let expect = Rootless::new("John.Doe@example.com".to_string()).unwrap();
    assert_eq!(x.scheme.get(), "mailto");
    assert_matches!(
        x.hier_part,
        HierPart::Rootless(expect)
    );
}

#[test]
fn parse_tel() {
    let input = "tel:+1-816-555-1212";
    let x = Uri::try_from(input).unwrap();
    let expect = Rootless::new("+1-816-555-1212".to_string()).unwrap();
    assert_eq!(x.scheme.get(), "tel");
    assert_matches!(
        x.hier_part,
        HierPart::Rootless(expect)
    );
}

#[test]
fn parse_urn() {
    let input = "urn:oasis:names:specification:docbook:dtd:xml:4.1.2";
    let x = Uri::try_from(input).unwrap();
    let expect = Rootless::new("oasis:names:specification:docbook:dtd:xml:4.1.2".to_string()).unwrap();
    assert_eq!(x.scheme.get(), "urn");
    assert_matches!(
        x.hier_part,
        HierPart::Rootless(expect)
    );
}

#[test]
fn parse_file() {
    let input = "file:///home/user/foo.txt";
    let x = Uri::try_from(input).unwrap();
    assert_eq!(x.scheme.get(), "file");
    assert_matches!(
        x.hier_part,
        HierPart::AbEmpty(auth, path) if
            auth.user_info.is_none() &&
            auth.host == "" &&
            auth.port.is_none() &&
            path.get() == "/home/user/foo.txt"
    );
}
