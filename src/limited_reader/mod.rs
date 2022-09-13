use std::io::Read;

#[derive(Debug)]
pub struct LimitedReader<R: Read> {
    remain: Option<usize>,
    r: R,
}

impl<R: Read> LimitedReader<R> {
    pub fn new(r: R, limit: Option<usize>) -> LimitedReader<R> {
        LimitedReader {
            remain: limit,
            r: r,
        }
    }
    pub fn set_limit(&mut self, s: usize) {
        self.remain = Some(s)
    }
    pub fn unset_limit(&mut self) {
        self.remain = None
    }

    pub fn get_ref(&self) -> &R {
        &self.r
    }

    pub fn get_mut(&mut self) -> &mut R {
        &mut self.r
    }

    pub fn inner(self) -> R {
        self.r
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
