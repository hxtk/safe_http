#![feature(buf_read_has_data_left)]
#![feature(string_remove_matches)]
#![feature(assert_matches)]

mod uri;
mod ep;

#[cfg(test)]
mod tests;

use std::boxed::Box;
use std::collections::HashMap;
use std::error::Error;
use std::io::{ BufRead, BufReader, Read, Write };
use std::net::{ SocketAddr, TcpListener, TcpStream };
use std::os::unix::io::{ AsRawFd, RawFd };
use std::result::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::vec::Vec;

use rayon::iter::IntoParallelIterator;
use rayon::iter::ParallelIterator;
use rayon::ThreadPoolBuilder;
use epoll::Events;

use ep::Epoll;

#[derive(Debug)]
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
            }
        }
    }

    fn set(&mut self, key: &str, value: &str) {
        let key = normalize_header_field(key);
        let value = normalize_header_value(value);
        match self.hs.get_mut(&key) {
            Some(v) => {
                v.clear();
                v.push(value);
            }
            None => {
                self.hs.insert(key, vec![value]);
            }
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
pub struct Response {}

pub trait Handler {
    fn serve_http(&self, r: Request) -> Response;
}

#[derive(Debug)]
enum Context<W: Write + AsRawFd, R: Read> {
    Listener(TcpListener),
    Conn(Conn<W, R>),
    Ref(RawFd),
}

impl<W: Write + AsRawFd, R: Read> AsRawFd for Context<W, R> {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::Listener(l) => l.as_raw_fd(),
            Self::Conn(c) => c.w.as_raw_fd(),
            Self::Ref(fd) => *fd,
        }
    }
}

#[derive(Debug)]
struct Conn<W: Write + AsRawFd, R: Read> {
    w: W,
    br: BufReader<LimitedReader<R>>,
    peer: SocketAddr,
}

pub struct Server<T: Handler + Send + Sync> {
    handler: T,
    max_header_bytes: usize,
    concurrency: usize,
}

impl<T: Handler + Send + Sync> Server<T> {
    pub fn new(h: T) -> Self {
        Server {
            handler: h,
            max_header_bytes: 1 << 20,
            concurrency: 32,
        }
    }

    pub fn listen_and_serve(&self, s: &str) -> Result<(), Box<dyn Error>> {
        let listener = TcpListener::bind(s)?;
        self.serve(listener)?;
        Ok(())
    }

    pub fn serve(&self, l: TcpListener) -> Result<(), Box<dyn Error>> {
        let epfd: Arc<Epoll<Context<TcpStream, TcpStream>>> = Arc::new(Epoll::new(false)?);
        let backoff = AtomicU64::new(0);
        epfd.add(Context::Listener(l), Events::EPOLLIN)?;

        ThreadPoolBuilder::new().num_threads(self.concurrency).build()?.install(|| {
            loop {
                let wait = epfd.wait(None);
                if let Err(e) = wait {
                    println!("Error awaiting epoll: {}.", e);
                    break;
                }
                wait.unwrap().into_par_iter().for_each(|(event, ctx)| {
                    match &mut *ctx.lock().unwrap() {
                        Context::Ref(_) => {
                            panic!("Unreachable Context::Ref");
                        }
                        Context::Listener(x) => {
                            match x.accept() {
                                Err(e) => {
                                    println!("Error accepting connection: {}.", e);
                                    return;
                                    // TODO: backoff logic.
                                },
                                Ok((s, p)) => {
                                    backoff.store(0, Ordering::Relaxed);
                                    let w = match s.try_clone() {
                                        Ok(x) => x,
                                        Err(e) => {
                                            println!("Error accepting connection from {}: {}", p, e);
                                            return;
                                        }
                                    };
                                    let br = BufReader::new(LimitedReader::new(s, None));
                                    let c = Conn{
                                        w: w,
                                        br: br,
                                        peer: p,
                                    };

                                    let res = epfd.add(
                                        Context::Conn(c),
                                        Events::EPOLLIN | Events::EPOLLRDHUP,
                                    );

                                    if let Err(e) = res {
                                        println!("Error monitoring: {}", e);
                                    }
                                }
                            }
                        },
                        Context::Conn(c) => {
                            if event.contains(Events::EPOLLRDHUP) {
                                if let Err(e) = epfd.remove(Context::Ref(c.w.as_raw_fd())) {
                                    println!("Error removing closed connection: {}.", e);
                                }
                                return;
                            }

                            match read_request(&mut c.br, self.max_header_bytes) {
                                Ok(_req) => {
                                    c.w.write(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: 13\r\n\r\nHello, World!").unwrap();
                                },
                                Err(e) => {
                                    c.w.write(b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n400 Bad Request").unwrap();
                                    println!("{}", e);
                                },
                            };
                        }
                    };
                });
            }
        });
        Ok(())
    }
}

#[derive(Debug)]
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

fn read_request<R: Read>(
    br: &mut BufReader<LimitedReader<R>>,
    limit: usize,
) -> Result<Response, Box<dyn Error>> {
    let mut buf = String::new();
    br.get_mut().set_limit(limit);
    br.read_line(&mut buf)?;
    buf.remove_matches("\r\n");

    let _ = parse_request_line(&buf)?;
    let mut headers = Headers::new();
    loop {
        buf.clear();
        br.get_mut().set_limit(limit);
        br.read_line(&mut buf)?;
        buf.remove_matches("\r\n");
        if buf == "" {
            break;
        }

        if let Some((key, value)) = buf.split_once(":") {
            headers.insert(key, value);
            Ok(())
        } else {
            Err("malformed header: ".to_owned() + &buf)
        }?;
    }
    br.get_mut().unset_limit();

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
