use std::io::Result;
use std::collections::HashMap;
use std::convert::TryInto;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::vec::Vec;
use epoll::{self, Event, Events, ControlOptions};

struct Epoll {
    epfd: OwnedFd,
    es: Arc<Mutex<HashMap<RawFd, u32>>>,
}

impl Epoll {
    fn new(b: bool) -> Result<Self> {
        let epfd = epoll::create(b)?;
        Ok(Self{
            epfd: unsafe { OwnedFd::from_raw_fd(epfd) },
            es: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn add<'a, T>(&self, t: &'a T, events: Events) -> Result<()> where &'a T: AsRawFd {
        let res = epoll::ctl(
            self.epfd.as_raw_fd(),
            ControlOptions::EPOLL_CTL_ADD,
            t.as_raw_fd(), 
            Event::new(events, 0),
        )?;

        let ones = events.bits().count_ones();
        let mut lock = self.es.lock().unwrap();
        lock.insert(t.as_raw_fd(), ones);
        Ok(())
    }

    fn remove<'a, T>(&self, t: &'a T) -> Result<()> where &'a T: AsRawFd {
        let res = epoll::ctl(
            self.epfd.as_raw_fd(),
            ControlOptions::EPOLL_CTL_DEL,
            t.as_raw_fd(), 
            Event::new(Events::empty(), 0),
        )?;

        let mut lock = self.es.lock().unwrap();
        lock.remove(&t.as_raw_fd());
        Ok(())
    }

    fn wait(&self, timeout: Option<Duration>) -> Result<Vec<Event>> {
        let timeout: i32 = match timeout {
            Some(d) => d.as_millis().try_into().unwrap_or(i32::MAX),
            None => -1,
        };

        let lock = self.es.lock().unwrap();
        let size = lock.values().fold(0, |a, x| a + x);
        let mut buf = vec![Event::new(Events::empty(), 0); size as usize];
        drop(lock);

        let count = epoll::wait(
            self.epfd.as_raw_fd(),
            timeout,
            buf.as_mut_slice(),
        )?;

        buf.truncate(count);

        Ok(buf)
    }
}
