#[cfg(test)]
mod tests;

use std::collections::HashMap;

pub struct Headers {
    hs: HashMap<String, Vec<String>>,
}

impl Headers {
    pub fn new() -> Headers {
        Headers { hs: HashMap::new() }
    }

    pub fn insert(&mut self, key: &str, value: &str) {
        let key = normalize_header_field(key);
        let value = normalize_header_value(value);
        match self.hs.get_mut(&key) {
            Some(v) => v.push(value),
            None => {
                self.hs.insert(key, vec![value]);
            }
        }
    }

    pub fn set(&mut self, key: &str, value: &str) {
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

    pub fn exists(&self, key: &str) -> bool {
        self.hs.get(key).is_some()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        if let Some(v) = self.hs.get(key) {
            match v.get(0) {
                Some(x) => Some(&x),
                None => None,
            }
        } else {
            None
        }
    }

    pub fn get_vec(&self, key: &str) -> Option<&Vec<String>> {
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
