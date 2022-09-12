use std::collections::HashMap;
use std::convert::TryInto;
use std::io::{ Error, ErrorKind, Result };
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::vec::Vec;
use epoll::{self, Event, Events, ControlOptions};

pub struct Epoll<C> {
    epfd: OwnedFd,
    es: Arc<Mutex<HashMap<RawFd, Arc<Mutex<C>>>>>,
}

impl<C: AsRawFd> Epoll<C> {
    pub fn new(b: bool) -> Result<Self> {
        let epfd = epoll::create(b)?;
        Ok(Self{
            epfd: unsafe { OwnedFd::from_raw_fd(epfd) },
            es: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn add(&self, c: C, events: Events) -> Result<()> {
        let fd = c.as_raw_fd();
        epoll::ctl(
            self.epfd.as_raw_fd(),
            ControlOptions::EPOLL_CTL_ADD,
            fd,
            Event::new(events, fd.try_into().unwrap()),
        )?;

        let mut lock = self.es.lock().unwrap();
        lock.insert(c.as_raw_fd(), Arc::new(Mutex::new(c)));
        Ok(())
    }

    pub fn remove(&self, c: C) -> Result<Option<Arc<Mutex<C>>>> {
        let fd = c.as_raw_fd();
        epoll::ctl(
            self.epfd.as_raw_fd(),
            ControlOptions::EPOLL_CTL_DEL,
            fd,
            Event::new(Events::empty(), 0),
        )?;

        let mut lock = self.es.lock().unwrap();
        Ok(lock.remove(&fd))
    }

    pub fn wait(&self, timeout: Option<Duration>) -> Result<Vec<(Events, Arc<Mutex<C>>)>> {
        let timeout: i32 = match timeout {
            Some(d) => d.as_millis().try_into().unwrap_or(i32::MAX),
            None => -1,
        };

        let mut buf = {
            let lock = self.es.lock().map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
            vec![Event::new(Events::empty(), 0); lock.len()]
        };

        let count = epoll::wait(
            self.epfd.as_raw_fd(),
            timeout,
            buf.as_mut_slice(),
        )?;
        buf.truncate(count);

        let mut res: Vec<(Events, Arc<Mutex<C>>)> = Vec::with_capacity(count);
        let lock = self.es.lock().map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
        for x in buf.iter() {
            let events = Events::from_bits_truncate(x.events);
            let data: RawFd = x.data.try_into().unwrap();
            if let Some(x) = lock.get(&data) {
                res.push((events, x.clone()));
            }
        }
        Ok(res)
    }
}
