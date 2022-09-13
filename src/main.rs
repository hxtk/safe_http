use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;

use rustls;
use rustls_pemfile::{self as pem, Item};

use http::{self, Server};

struct EmptyHandler {}

impl http::Handler for EmptyHandler {
    fn serve_http(&self, _r: http::Request) -> http::Response {
        http::Response {}
    }
}

fn config() -> rustls::ServerConfig {
    let cert = {
        let mut br = BufReader::new(File::open("tls.crt").expect("tls.crt file did not open"));
        let cert = pem::read_one(&mut br)
            .expect("tls.crt did not read")
            .expect("tls.crt did not parse");
        match cert {
            Item::X509Certificate(x) => vec![rustls::Certificate(x)],
            x => panic!("Expected X509Certificate; got {:?}", x),
        }
    };

    let key = {
        let mut br = BufReader::new(File::open("tls.key").expect("tls.key file did not open"));
        let key = pem::read_one(&mut br)
            .expect("tls.key did not read")
            .expect("tls.key did not parse");
        match key {
            Item::RSAKey(x) => rustls::PrivateKey(x),
            Item::ECKey(x) => rustls::PrivateKey(x),
            Item::PKCS8Key(x) => rustls::PrivateKey(x),
            x => panic!("Expected key, got {:?}", x),
        }
    };

    rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert, key)
        .expect("bad certificate/key")
}

fn main() {
    let res = http::PollingServer::new(EmptyHandler {})
        .listen_and_serve_tls("0.0.0.0:8443", Arc::new(config()));
    //.listen_and_serve("0.0.0.0:8080");
    match res {
        Ok(_) => {
            println!("Server exited cleanly");
        }
        Err(e) => {
            println!("Server error: {}", e);
        }
    }
    println!("Hello, World")
}
