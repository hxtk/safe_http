#![feature(buf_read_has_data_left)]
#![feature(string_remove_matches)]
#![feature(assert_matches)]

mod uri;

#[cfg(test)]
mod tests;

use uri::Uri;
use rayon;
use std::boxed::Box;
use std::cmp;
use std::collections::HashMap;
use std::error::Error;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener};
use std::result::Result;
use std::thread;
use std::time::Duration;
use std::vec::Vec;


pub enum Method {
    Options,
    Get,
    Head,
    Post,
    Put,
    Delete,
    Trace,
    Connect,
    Extension(String),
}

impl std::convert::From<&str> for Method {
    fn from(s: &str) -> Self {
        match s {
            "OPTIONS" => Self::Options,
            "GET" => Self::Get,
            "HEAD" => Self::Head,
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "DELETE" => Self::Delete,
            "TRACE" => Self::Trace,
            "CONNECT" => Self::Connect,
            s => Self::Extension(String::from(s)),
        }
    }
}

pub struct Headers {
    hs: HashMap<String, Vec<String>>,
}

impl Headers {
    fn new() -> Headers {
        Headers { hs: HashMap::new() }
    }

    fn insert(&mut self, key: &str, value: &str) {
        let key = normalize_header_field(key);
        let value = normalize_header_value(value);
        match self.hs.get_mut(&key) {
            Some(v) => v.push(value),
            None => {
                self.hs.insert(key, vec![value]);
            },
        }
    }

    fn set(&mut self, key: &str, value: &str) {
        let key = normalize_header_field(key);
        let value = normalize_header_value(value);
        match self.hs.get_mut(&key) {
            Some(v) => {
                v.clear();
                v.push(value);
            },
            None => {
                self.hs.insert(key, vec![value]);
            },
        }
    }

    fn exists(&self, key: &str) -> bool {
        self.hs.get(key).is_some()
    }

    fn get(&self, key: &str) -> Option<&str> {
        if let Some(v) = self.hs.get(key) {
            match v.get(0) {
                Some(x) => Some(&x),
                None => None,
            }
        } else {
            None
        }
    }

    fn get_vec(&self, key: &str) -> Option<&Vec<String>> {
        if let Some(v) = self.hs.get(key) {
            Some(&v)
        } else {
            None
        }
    }
}

static GENERAL_HDRS: [&str; 9] = [
    "Cache-Control",
    "Connection",
    "Date",
    "Pragma",
    "Trailer",
    "Transfer-Encoding",
    "Upgrade",
    "Via",
    "Warning",
];

static REQUEST_HDRS: [&str; 19] = [
    "Accept",
    "Accept-Charset",
    "Accept-Encoding",
    "Accept-Language",
    "Authorization",
    "Expect",
    "From",
    "Host",
    "If-Match",
    "If-Modified-Since",
    "If-None-Match",
    "If-Range",
    "If-Unmodified-Since",
    "Max-Forwards",
    "Proxy-Authorization",
    "Range",
    "Referer",
    "TE",
    "User-Agent",
];

static ENTITY_HDRS: [&str; 9] = [
    "Allow",
    "Content-Encoding",
    "Content-Language",
    "Content-Length",
    "Content-MD5",
    "Content-Range",
    "Content-Type",
    "Expires",
    "Last-Modified",
];

impl std::convert::Into<Vec<u8>> for Headers {
    fn into(mut self) -> Vec<u8> {
        let mut res = String::new();
        for k in GENERAL_HDRS {
            if let Some(v) = self.hs.remove(k) {
                v.iter().for_each(|x| {
                    res.push_str(&k);
                    res.push_str(": ");
                    res.push_str(x);
                    res.push_str("\r\n");
                });
            }
        }
        for k in REQUEST_HDRS {
            if let Some(v) = self.hs.remove(k) {
                v.iter().for_each(|x| {
                    res.push_str(&k);
                    res.push_str(": ");
                    res.push_str(x);
                    res.push_str("\r\n");
                });
            }
        }
        for k in ENTITY_HDRS {
            if let Some(v) = self.hs.remove(k) {
                v.iter().for_each(|x| {
                    res.push_str(&k);
                    res.push_str(": ");
                    res.push_str(x);
                    res.push_str("\r\n");
                });
            }
        }
        for (k, v) in self.hs {
            v.iter().for_each(|x| {
                res.push_str(&k);
                res.push_str(": ");
                res.push_str(x);
                res.push_str("\r\n");
            });
        }

        res.into_bytes()
    }
}

fn normalize_header_field(s: &str) -> String {
    s.split('-')
        .map(|x| {
            let (a, b) = x.split_at(1);
            a.to_uppercase() + &b.to_lowercase()
        })
        .fold(String::new(), |acc, x| acc + "-" + &x)[1..]
        .to_string()
}

fn normalize_header_value(s: &str) -> String {
    s.split_ascii_whitespace()
        .fold(String::new(), |acc, x| acc + " " + x)
        .trim()
        .to_string()
}

pub struct Request {
    //pub uri: Uri,
    pub headers: Headers,
}

#[derive(Debug)]
pub struct Response {
}

pub trait Handler {
    fn serve_http(&self, r: Request) -> Response;
}

pub struct Server<T: Handler + Send + Sync> {
    handler: T,
    max_header_bytes: usize,
}

impl<T: Handler + Send + Sync> Server<T> {
    pub fn new(h: T) -> Self {
        Server {
            handler: h,
            max_header_bytes: 1 << 20,
        }
    }

    pub fn listen_and_serve(&self, s: &str) -> Result<(), Box<dyn Error>> {
        let listener = TcpListener::bind(s)?;
        self.serve(listener)?;
        Ok(())
    }

    pub fn serve(&self, l: TcpListener) -> Result<(), Box<dyn Error>> {
        let mut backoff = Duration::from_millis(0);
        rayon::scope(|s| {
            for stream in l.incoming() {
                if let Err(_) = stream {
                    backoff = if backoff.is_zero() {
                        Duration::from_millis(5)
                    } else {
                        cmp::min(Duration::from_millis(1000), backoff.saturating_mul(2))
                    };
                    thread::sleep(backoff);
                    continue;
                }
                // Unwrap is safe because we caught the error above.
                let mut stream = stream.unwrap();

                backoff = Duration::from_millis(0);
                s.spawn(move |_| {
                    loop {
                        let mut br = BufReader::new(LimitedReader::new(&stream, Some(self.max_header_bytes)));
                        match br.has_data_left() {
                            Ok(true) => (),
                            _ => {
                                break;
                            },
                        }
                        match read_request(&mut br) {
                            Ok(req) => {
                        stream.write(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: 13\r\n\r\nHello, World!").unwrap();
                            },
                            Err(e) => {
                                stream.write(b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n400 Bad Request").unwrap();
                                println!("{}", e);
                                break;
                            }
                        }

                    }
                });
            }
        });
        Ok(())
    }
}

struct LimitedReader<R: Read> {
    remain: Option<usize>,
    r: R,
}

impl<R: Read> LimitedReader<R> {
    fn new(r: R, limit: Option<usize>) -> LimitedReader<R> {
        LimitedReader {
            remain: limit,
            r: r,
        }
    }
    fn set_limit(&mut self, s: usize) {
        self.remain = Some(s)
    }
    fn unset_limit(&mut self) {
        self.remain = None
    }
}

impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.remain {
            None => self.r.read(buf),
            Some(remain) => {
                let buf = if remain < buf.len() {
                    &mut buf[..remain]
                } else {
                    buf
                };
                let res = self.r.read(buf)?;
                self.remain = Some(remain - res);
                Ok(res)
            }
        }
    }
}

fn read_request<R: BufRead>(mut br: R) -> Result<Response, Box<dyn Error>> {
    let mut buf = String::new();
    br.read_line(&mut buf)?;
    buf.remove_matches("\r\n");

    let (method, uri, version) = parse_request_line(&buf)?;

    let mut headers = Headers::new();
    loop {
        buf.clear();
        br.read_line(&mut buf)?;
        buf.remove_matches("\r\n");
        if buf == "" {
            break
        }

        if let Some((key, value)) = buf.split_once(":") {
            headers.insert(key, value);
            Ok(())
        } else {
            Err("malformed header: ".to_owned() + &buf)
        }?;
    }

    Ok(Response {})
}

fn parse_request_line(rl: &str) -> Result<(Method, String, String), Box<dyn Error>> {
    let mut parts = rl.split_whitespace();
    let method = if let Some(method_str) = parts.next() {
        Ok(Method::from(method_str))
    } else {
        Err("no method")
    }?;

    let uri = if let Some(uri_str) = parts.next() {
        Ok(uri_str)
    } else {
        Err("no uri")
    }?;

    let version = if let Some(version_str) = parts.next() {
        Ok(version_str)
    } else {
        Err("no version")
    }?;

    Ok(if let Some(_) = parts.next() {
        Err("too many parts")
    } else {
        Ok((method, String::from(uri), String::from(version)))
    }?)
}
