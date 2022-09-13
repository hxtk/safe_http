#![feature(buf_read_has_data_left)]
#![feature(string_remove_matches)]
#![feature(assert_matches)]
#![feature(mutex_unpoison)]

mod ep;
mod headers;
mod limited_reader;
mod uri;

#[cfg(test)]
mod tests;

use std::boxed::Box;
use std::error::Error;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::io::{AsRawFd, RawFd};
use std::result::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError, Weak};
use std::thread;
use std::time::Duration;

use epoll::Events;

use ep::Epoll;
use headers::Headers;
use limited_reader::LimitedReader;

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

pub struct Request {
    //pub uri: Uri,
    pub headers: Headers,
}

#[derive(Debug)]
pub struct Response {}

pub trait Handler {
    fn serve_http(&self, r: Request) -> Response;
}

pub trait Server {
    fn serve(&self, l: TcpListener) -> Result<(), Box<dyn Error>>;

    fn listen_and_serve(&self, s: &str) -> Result<(), Box<dyn Error>> {
        let listener = TcpListener::bind(s)?;
        self.serve(listener)?;
        Ok(())
    }
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

pub struct TPCServer<T: Handler + Send + Sync> {
    handler: T,
    max_header_bytes: usize,
}

impl<T: Handler + Send + Sync> TPCServer<T> {
    pub fn new(h: T) -> Self {
        Self {
            handler: h,
            max_header_bytes: 1 << 20,
        }
    }
}

impl<T: Handler + Send + Sync> Server for TPCServer<T> {
    fn serve(&self, l: TcpListener) -> Result<(), Box<dyn Error>> {
        let mut backoff = Duration::from_millis(0);
        thread::scope(|s| {
            for stream in l.incoming() {
                if let Err(_) = stream {
                    backoff = if backoff.is_zero() {
                        Duration::from_millis(5)
                    } else {
                        std::cmp::min(Duration::from_millis(1000), backoff.saturating_mul(2))
                    };
                    thread::sleep(backoff);
                    continue;
                }

                // Unwrap is safe because we caught the error above.
                let stream = stream.unwrap();
                backoff = Duration::from_millis(0);
                s.spawn(move || {
                    // TODO: Move this block before the thread spawn.
                    // On error, we should be backing off accepting
                    // new connections.
                    let mut w = match stream.try_clone() {
                        Ok(x) => x,
                        Err(_) => return,
                    };
                    let mut br = BufReader::new(LimitedReader::new(&stream, None));
                    loop {
                        match br.has_data_left() {
                            Ok(true) => (),
                            _ => {
                                break;
                            },
                        };
                        match read_request(&mut br, self.max_header_bytes) {
                            Ok(req) => {
                        w.write(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: 13\r\n\r\nHello, World!").unwrap();
                            },
                            Err(e) => {
                                w.write(b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n400 Bad Request").unwrap();
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

pub struct PollingServer<T: Handler + Send + Sync> {
    handler: T,
    max_header_bytes: usize,
    concurrency: usize,
}

impl<T: Handler + Send + Sync> PollingServer<T> {
    pub fn new(h: T) -> Self {
        Self {
            handler: h,
            max_header_bytes: 1 << 20,
            concurrency: 32,
        }
    }
}

impl<T: Handler + Send + Sync> Server for PollingServer<T> {
    fn serve(&self, l: TcpListener) -> Result<(), Box<dyn Error>> {
        l.set_nonblocking(true)?;

        let lock = Arc::new(Mutex::new(()));
        let epfd = Arc::new(Epoll::new(false)?);
        let backoff = Arc::new(AtomicU64::new(0));
        epfd.add(Context::Listener(l), Events::EPOLLIN)?;
        thread::scope(|s| {
            for _ in 0..self.concurrency {
                let lock = Arc::clone(&lock);
                let epfd = Arc::clone(&epfd);
                let backoff = Arc::clone(&backoff);
                let header_bytes = self.max_header_bytes;
                s.spawn(
                    move || match server_loop(lock, epfd, backoff, header_bytes) {
                        Ok(_) => println!("{:?} exited silently.", thread::current().id()),
                        Err(e) => println!("{:?} errored: {}.", thread::current().id(), e),
                    },
                );
            }
        });
        Ok(())
    }
}

fn server_loop(
    lock: Arc<Mutex<()>>,
    epfd: Arc<Epoll<Context<TcpStream, TcpStream>>>,
    backoff: Arc<AtomicU64>,
    max_header_bytes: usize,
) -> Result<(), Box<dyn Error>> {
    loop {
        let (event, ctx) = {
            let _guard = lock.lock().unwrap_or_else(|e| {
                lock.clear_poison();
                e.into_inner()
            });

            match epfd.wait_one(None)? {
                Some(x) => x,
                None => continue,
            }
        };

        let mut lock = match ctx.try_lock() {
            Err(TryLockError::WouldBlock) => {
                // Some other thread is currently processing this event.
                continue;
            }
            Err(TryLockError::Poisoned(g)) => {
                let inner = g.into_inner();
                match &*inner {
                    Context::Ref(_) => panic!("Unreachable"),
                    Context::Listener(_) => {
                        // A listener consists only of a TcpSocket,
                        // which is thread-safe anyway, so we clear
                        // the poison.
                        ctx.clear_poison();
                        inner
                    }
                    Context::Conn(c) => {
                        // If a client connection became poisoned,
                        // all we can do is drop it. The context
                        // may not be well-defined.
                        // 
                        // TODO: We may be able to get away with
                        // clearing the buffer on the BufReader
                        // and requiring any future client state
                        // to be Sync.
                        close(c.w.as_raw_fd(), Arc::clone(&epfd));
                        continue;
                    }
                }
            }
            Ok(lock) => lock,
        };

        match &mut *lock {
            Context::Ref(_) => {
                panic!("Unreachable Context::Ref");
            }
            Context::Listener(x) => match x.accept() {
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    println!("Error accepting connection: {}.", e);
                    backoff_helper(
                        Arc::downgrade(&backoff),
                        Arc::downgrade(&epfd),
                        x.as_raw_fd(),
                    );
                    continue;
                }
                Ok((s, p)) => {
                    backoff.store(0, Ordering::Relaxed);
                    let w = match s.try_clone() {
                        Ok(x) => x,
                        Err(e) => {
                            println!("Error accepting connection from {}: {}", p, e);
                            backoff_helper(
                                Arc::downgrade(&backoff),
                                Arc::downgrade(&epfd),
                                x.as_raw_fd(),
                            );
                            continue;
                        }
                    };
                    let br = BufReader::new(LimitedReader::new(s, None));
                    let c = Conn {
                        w: w,
                        br: br,
                        peer: p,
                    };

                    let res = epfd.add(Context::Conn(c), Events::EPOLLIN | Events::EPOLLRDHUP);

                    if let Err(e) = res {
                        println!("Error monitoring: {}", e);
                    }
                }
            },
            Context::Conn(c) => {
                if event.contains(Events::EPOLLRDHUP) {
                    close(c.w.as_raw_fd(), Arc::clone(&epfd));
                    continue;
                }

                c.br.get_ref().get_ref().set_nonblocking(true)?;
                match c.br.has_data_left() {
                    Ok(true) => (),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        continue;
                    }
                    _ => {
                        close(c.w.as_raw_fd(), Arc::clone(&epfd));
                        continue;
                    }
                };
                c.br.get_ref().get_ref().set_nonblocking(false)?;
                match read_request(&mut c.br, max_header_bytes) {
                    Ok(_req) => {
                        if let Err(e) = c.w.write(HELLO) {
                            println!("Error sending reply: {}.", e);
                        }
                    }
                    Err(e) => {
                        println!("Error parsing request: {}.", e);
                        if let Err(e) = c.w.write(BAD_REQUEST) {
                            if e.kind() == ErrorKind::BrokenPipe {
                                close(c.w.as_raw_fd(), Arc::clone(&epfd));
                                continue;
                            }
                        }
                    }
                };
            }
        };
    }
}

fn close(fd: RawFd, epfd: Arc<Epoll<Context<TcpStream, TcpStream>>>) {
    if let Err(e) = epfd.remove(Context::Ref(fd)) {
        println!("Error removing closed connection: {}.", e);
    }
}

static HELLO: &[u8; 92] = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: 13\r\n\r\nHello, World!";

static BAD_REQUEST: &[u8; 103] = b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n400 Bad Request";

fn backoff_helper(
    bo: Weak<AtomicU64>,
    epfd: Weak<Epoll<Context<TcpStream, TcpStream>>>,
    fd: RawFd,
) {
    let dur = match bo.upgrade() {
        None => return,
        Some(bo) => {
            let tmp = match bo.load(Ordering::Relaxed) {
                0 => 5,
                x if x < 500 => x * 2,
                _ => 1000,
            };
            bo.store(tmp, Ordering::Release);

            Duration::from_millis(tmp)
        }
    };

    println!(
        "Backing off accepting new connections for {}ms",
        dur.as_millis()
    );
    thread::spawn(move || {
        if let Some(epfd) = epfd.upgrade() {
            match epfd.modify(Context::Ref(fd), Events::empty()) {
                Ok(_) => {}
                Err(e) => {
                    println!("Error modifying events for listener: {}.", e);
                    return;
                }
            }
        }
        thread::sleep(dur);
        if let Some(epfd) = epfd.upgrade() {
            match epfd.modify(Context::Ref(fd), Events::EPOLLIN) {
                Ok(_) => {}
                Err(e) => {
                    println!("Error modifying events for listener: {}.", e);
                    return;
                }
            }
        }
    });
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
