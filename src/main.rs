use http::{self, Server};

struct EmptyHandler {}

impl http::Handler for EmptyHandler {
    fn serve_http(&self, _r: http::Request) -> http::Response {
        http::Response {}
    }
}

fn main() {
    let res = http::TPCServer::new(EmptyHandler {}).listen_and_serve("0.0.0.0:8080");
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
