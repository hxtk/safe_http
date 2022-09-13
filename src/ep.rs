use epoll::{self, ControlOptions, Event, Events};
use std::cmp::max;
use std::collections::HashMap;
use std::convert::TryInto;
use std::io::{Error, ErrorKind, Result};
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::vec::Vec;

/// Epoll is an RAII wrapper over an epoll file descriptor.
///
/// Epoll operations deal with Context objects addressed
/// by the type parameter C. These objects must implement
/// `AsRawFd` so that their file descriptor may be passed
/// to the underlying epoll APIs.
///
/// The epoll file descriptor is automatically closed when
/// this struct goes out of scope.
pub struct Epoll<C> {
    epfd: OwnedFd,
    es: Arc<Mutex<HashMap<RawFd, Arc<Mutex<C>>>>>,
}

impl<C: AsRawFd> Epoll<C> {
    /// Create a new Epoll object using epoll_create1.
    pub fn new(cloexec: bool) -> Result<Self> {
        let epfd = epoll::create(cloexec)?;
        Ok(Self {
            epfd: unsafe { OwnedFd::from_raw_fd(epfd) },
            es: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Add the file descriptor for `c` to the epoll list.
    ///
    /// The context object `c` is saved internally and will
    /// be returned alongside the epoll event data from
    /// `wait`.
    ///
    /// The events bitfield indicates the events for which
    /// this file descriptor shall be monitored by the epoll
    /// file descriptor.
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

    /// Modify the events being monitored for `C`.
    ///
    /// Only the file descriptor for `C` is used; the stored
    /// context object for `C` is updated by this operation.
    pub fn modify(&self, c: C, events: Events) -> Result<()> {
        let fd = c.as_raw_fd();
        epoll::ctl(
            self.epfd.as_raw_fd(),
            ControlOptions::EPOLL_CTL_MOD,
            fd,
            Event::new(events, fd.try_into().unwrap()),
        )?;
        Ok(())
    }

    /// Remove `c` from the list of monitored file descriptors.
    ///
    /// If `c` is monitored by this Epoll, this method will delete
    /// it from the Epoll list and return the stored context for
    /// that file descriptor.
    ///
    /// If the epoll_ctl call results in an error, that error will
    /// generally be passed, with the exception of `ENOENT`. If
    /// `ENOENT` occurs, meaning epoll was not monitoring the given
    /// file descriptor, generally `Ok(None)` will be returned.
    pub fn remove(&self, c: C) -> Result<Option<Arc<Mutex<C>>>> {
        let fd = c.as_raw_fd();
        let res = epoll::ctl(
            self.epfd.as_raw_fd(),
            ControlOptions::EPOLL_CTL_DEL,
            fd,
            Event::new(Events::empty(), 0),
        );

        match res {
            Err(e) if e.kind() == ErrorKind::NotFound => {
                let mut lock = self.es.lock().unwrap();
                Ok(lock.remove(&fd))
            }
            Ok(_) => {
                let mut lock = self.es.lock().unwrap();
                Ok(lock.remove(&fd))
            }
            Err(e) => {
                let mut lock = self.es.lock().unwrap();
                lock.remove(&fd);
                Err(e)
            }
        }
    }

    /// Wait for the Epoll to indicate one of its monitored files is ready.
    ///
    /// Timeout indicates the duration for which Epoll should wait for a result.
    /// A timeout of None will cause epoll to block indefinitely.
    ///
    /// The return value will be a vector of tuples for all file descriptors that
    /// had events to report, including the bitfield of triggered `Events` and a
    /// strong reference to the stored context object for that file descriptor.
    pub fn wait(&self, timeout: Option<Duration>) -> Result<Vec<(Events, Arc<Mutex<C>>)>> {
        let timeout: i32 = match timeout {
            Some(d) => d.as_millis().try_into().unwrap_or(i32::MAX),
            None => -1,
        };

        let mut buf = {
            let lock = self
                .es
                .lock()
                .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
            vec![Event::new(Events::empty(), 0); max(lock.len(), 1)]
        };

        let count = epoll::wait(self.epfd.as_raw_fd(), timeout, buf.as_mut_slice())?;
        buf.truncate(count);

        let mut res: Vec<(Events, Arc<Mutex<C>>)> = Vec::with_capacity(count);
        let lock = self
            .es
            .lock()
            .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
        for x in buf.iter() {
            let events = Events::from_bits_truncate(x.events);
            let data: RawFd = x.data.try_into().unwrap();
            if let Some(x) = lock.get(&data) {
                res.push((events, x.clone()));
            }
        }
        Ok(res)
    }

    /// Wait for the Epoll to indicate one of its monitored files is ready.
    ///
    /// Timeout indicates the duration for which Epoll should wait for a result.
    /// A timeout of None will cause epoll to block indefinitely.
    ///
    /// The return value will be a vector of tuples for all file descriptors that
    /// had events to report, including the bitfield of triggered `Events` and a
    /// strong reference to the stored context object for that file descriptor.
    pub fn wait_one(&self, timeout: Option<Duration>) -> Result<Option<(Events, Arc<Mutex<C>>)>> {
        let timeout: i32 = match timeout {
            Some(d) => d.as_millis().try_into().unwrap_or(i32::MAX),
            None => -1,
        };

        let mut buf = [Event::new(Events::empty(), 0); 1];
        let count = epoll::wait(self.epfd.as_raw_fd(), timeout, &mut buf)?;
        if count == 0 {
            return Ok(None);
        }

        let lock = self
            .es
            .lock()
            .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;

        let events = Events::from_bits_truncate(buf[0].events);
        let data: RawFd = buf[0].data.try_into().unwrap();
        match lock.get(&data) {
            Some(x) => Ok(Some((events, Arc::clone(x)))),
            None => Ok(None)
        }
    }
}
