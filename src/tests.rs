use super::LimitedReader;
use std::io::BufReader;

#[test]
fn it_works() {
    assert_eq!(2 + 2, 4);
}

#[test]
fn normalize_header_value_with_crlf() {
    let got = super::normalize_header_value("Foo Bar\r\nBaz: Foo");
    assert_eq!(got, "Foo Bar Baz: Foo");
}

#[test]
fn read_request() {
    let req_str: &[u8] =
        b"GET / HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nUser-Agent: curl/7.82.0\r\nAccept: */*\r\n\r\n";
    let mut br = BufReader::new(LimitedReader::new(req_str, None));
    let res = super::read_request(&mut br, 1 << 20);
    println!("{:#?}", res);
    assert!(res.is_ok());
}

#[test]
fn read_consecutive_request() {
    let req_str: &[u8] = b"GET / HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nUser-Agent: curl/7.82.0\r\nAccept: */*\r\n\r\nGET / HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nUser-Agent: curl/7.82.0\r\nAccept: */*\r\n\r\n";
    let mut stream = BufReader::new(LimitedReader::new(req_str, None));
    let _ = super::read_request(&mut stream, 1 << 20);
    let res = super::read_request(&mut stream, 1 << 20);
    println!("{:#?}", res);
    assert!(res.is_ok());
}
