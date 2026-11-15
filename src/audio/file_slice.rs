use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
pub struct FileSlice {
    file: File,
    start: u64,
    len: u64,
    pos: u64,
}
impl FileSlice {
    pub(super) fn new(mut file: File, start: u64, len: u64) -> std::io::Result<Self> {
        file.seek(SeekFrom::Start(start))?;
        Ok(Self {
            file,
            start,
            len,
            pos: 0,
        })
    }
}
impl Read for FileSlice {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }
        let max = (self.len - self.pos).min(buf.len() as u64) as usize;
        let n = self.file.read(&mut buf[..max])?;
        self.pos += n as u64;
        Ok(n)
    }
}
impl Seek for FileSlice {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(d) => self.len as i64 + d,
            SeekFrom::Current(d) => self.pos as i64 + d,
        };
        if target < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before start of slice",
            ));
        }
        let target = target as u64;
        self.file.seek(SeekFrom::Start(self.start + target))?;
        self.pos = target;
        Ok(target)
    }
}
