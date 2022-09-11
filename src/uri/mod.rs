#[cfg(test)]
mod tests;

use std::boxed::Box;
use std::result::Result;
use std::collections::BTreeMap;
use std::vec::Vec;
use std::num::NonZeroU32;
use regex::Regex;
use once_cell::sync::Lazy;
use tightness::bound;

pub enum UriRef {
    Uri(Uri),
    Relative(RelativeUri),
}

pub struct RelativeUri {
    relative_part: RelativePart,
    query: Option<Query>,
    fragment: Option<String>,
}

enum RelativePart {
    AbEmpty(Authority, AbEmpty),
    Absolute(Absolute),
    NoScheme(NoScheme),
    Empty,
}

#[derive(Debug)]
pub struct Uri {
    scheme: Scheme,
    hier_part: HierPart,
    query: Option<Query>,
    fragment: Option<String>,
}

impl std::convert::TryFrom<&str> for Uri {
    type Error = Box<dyn std::error::Error>;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        let (scheme, remain) = s.split_once(':').ok_or("uri must contain at least one ':'")?;
        let scheme = Scheme::new(scheme.to_string())?;

        let (hier_part, remain) = if let Some(idx) = remain.find('?') {
            remain.split_at(idx)
        } else if let Some(idx) = remain.find('#') {
            remain.split_at(idx)
        } else {
            (remain, "")
        };

        let hier_part = if hier_part.starts_with("//") {
            let (authority, path) = hier_part[2..]
                .find('/')
                .map(|mid| hier_part[2..].split_at(mid))
                .unwrap_or((&hier_part[2..], ""));

            let (user_info, host) = match authority.split_once('@') {
                Some((u, h)) => (Some(u.to_owned()), h),
                None => (None, authority),
            };

            let (host, port) = if let Some(idx) = host.find(']') {
                if idx+1 == host.len() {
                    (host.to_owned(), None)
                } else {
                    let (h, p) = host.split_at(idx+1);
                    let p = u32::from_str_radix(p, 10)?;
                    match NonZeroU32::new(p) {
                        Some(nzp) => Ok((h.to_owned(), Some(nzp))),
                        None => Err("port cannot be zero"),
                    }?
                }
            } else {
                host.split_once(':')
                    .map(|(h, p)| (h.to_owned(), u32::from_str_radix(p, 10)))
                    .map(|(h, r)| match r {
                        Ok(x) => Ok((h, x)),
                        Err(e) => Err(e),
                    })
                    .map(|r| match r {
                        Err(_) => Err("error parsing port number"),
                        Ok((h, p)) => match NonZeroU32::new(p) {
                            Some(nzp) => Ok((h, Some(nzp))),
                            None => Err("port must be nonzero if specified"),
                        },
                    }).unwrap_or(Ok((host.to_owned(), None)))?
            };

            let path = AbEmpty::new((*path).to_string())?;

            HierPart::AbEmpty(Authority{
                user_info: user_info,
                host: host,
                port: port,
            }, path)
        } else if hier_part.is_empty() {
            HierPart::Empty
        } else if hier_part.starts_with('/') {
            HierPart::Absolute(Absolute::new(hier_part.to_string())?)
        } else {
            HierPart::Rootless(Rootless::new(hier_part.to_string())?)
        };

        let (query, remain) = if !remain.starts_with('?') {
            (None, remain)
        } else {
            let (qs, remain) = if let Some(idx) = remain.find('#') {
                remain[1..].split_at(idx)
            } else {
                (&remain[1..], "")
            };
            
            let mut query = Query::new();
            for x in qs.split('&') {
                match x.split_once('=') {
                    Some((l, r)) => query.add(l, Some(r)),
                    None => query.add(x, None),
                }
            }

            (Some(query), remain)
        };

        if remain.starts_with('#') {
            Ok(Uri{
                scheme: scheme,
                hier_part: hier_part,
                query: query,
                fragment: Some(remain[1..].to_string()),
            })
        } else {
            Ok(Uri{
                scheme: scheme,
                hier_part: hier_part,
                query: query,
                fragment: None,
            })
        }
    }
}

impl std::string::ToString for Uri {
    fn to_string(&self) -> String {
        let mut res = match &self.hier_part {
            HierPart::Empty => self.scheme.get().to_owned() + ":",
            HierPart::Absolute(p) => self.scheme.get().to_owned() + ":" + &p,
            HierPart::Rootless(p) => self.scheme.get().to_owned() + ":" + &p,
            HierPart::AbEmpty(a, p) => {
                let mut res = self.scheme.get().to_owned() + "://";
                if let Some(user_info) = &a.user_info {
                    res.push_str(&user_info);
                    res.push('@');
                }
                res += &a.host;
                if let Some(port) = &a.port {
                    res.push(':');
                    res.push_str(&port.to_string());
                }
                res.push_str(p);

                res
            }
        };

        if let Some(q) = &self.query {
            res += "?";
            res = res + &q.to_string();
        }

        if let Some(f) = &self.fragment {
            res += "#";
            res = res + f;
        }

        res
    }
}

#[derive(Debug)]
enum HierPart {
    AbEmpty(Authority, AbEmpty),
    Absolute(Absolute),
    Rootless(Rootless),
    Empty,
}

#[derive(Debug)]
struct Authority {
    user_info: Option<String>,
    host: Host,
    port: Option<NonZeroU32>,
}

enum Path {
    Empty,
    Rootless(Rootless),
    NoScheme(NoScheme),
    Absolute(Absolute),
    AbEmpty(AbEmpty),
}

static SCHEME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-zA-Z][a-zA-Z0-9+-.]*$").unwrap());

bound!(Scheme: String where |s| SCHEME_RE.is_match(s));

bound!(Rootless: String where |s| {
    !s.split_once('/').unwrap_or((&s, "")).0.is_empty()
});

bound!(NoScheme: String where |s| {
    !s.split_once('/').unwrap_or((&s, "")).0.contains(':')
});

bound!(Absolute: String where |s| {
    s.starts_with('/') && !s.starts_with("//")
});

bound!(AbEmpty: String where |s| {
    s.is_empty() || s.starts_with('/')
});

impl std::string::ToString for Path {
    fn to_string(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Rootless(x) => x.to_string(),
            Self::NoScheme(x) => x.to_string(),
            Self::Absolute(x) => x.to_string(),
            Self::AbEmpty(x) => x.to_string(),
        }
    }
}

#[derive(Debug)]
struct Query {
    qs: BTreeMap<String, Vec<Option<String>>>,
}

impl Query {
    fn new() -> Query {
        Query { qs: BTreeMap::new() }
    }

    fn add(&mut self, key: &str, value: Option<&str>) {
        let key = key.to_owned();
        let value = value.map(|x| x.to_owned());
        match self.qs.get_mut(&key) {
            Some(v) => v.push(value),
            None => {
                self.qs.insert(key, vec![value]);
            },
        }
    }

    fn set(&mut self, key: &str, value: Option<&str>) {
        let key = key.to_owned();
        let value = value.map(|x| x.to_owned());
        match self.qs.get_mut(&key) {
            Some(v) => {
                v.clear();
                v.push(value);
            },
            None => {
                self.qs.insert(key, vec![value]);
            },
        }
    }
}

impl std::string::ToString for Query {
    fn to_string(&self) -> String {
        let mut vs = Vec::<String>::new();
        self.qs.iter()
            .for_each(|(k, v)| {
                v.iter().for_each(|x| {
                    match x {
                        Some(x) => {vs.push(k.to_owned() + "=" + x);},
                        None => {vs.push(k.to_owned());},
                    }
                });
            });
        vs.iter().fold(String::new(), |a, x| a + "&" + x)[1..].to_string()
    }
}

type Host = String;
