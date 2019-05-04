/// Parse logs
use std::io::BufRead;

type Fallible<T> = crate::Fallible<T>;

#[derive(Debug, Eq, PartialEq)]
pub struct Entry<'a> {
    pub date: &'a str,
    pub time: &'a str,
    pub nick: String,
    pub msg: &'a str,
}

pub struct Parser<R: BufRead> {
    r: R,
    buf: String,
}

pub fn normalize_nick(s: &str) -> String {
    s.trim().trim_matches(|c| c == '@' || c == '>').to_ascii_lowercase()
}

impl<'a> Entry<'a> {
    /// Parse an entry from a line
    pub fn from_line(line: &'a str) -> Option<Self> {
        let mut splitter = line.splitn(4, |c: char| c.is_ascii_whitespace());
        let date = splitter.next()?;
        let time = splitter.next()?;
        let nick = normalize_nick(splitter.next()?);
        let msg = splitter.next()?;
        if nick == "-->" || nick == "<--" || nick == "--" {
            None
        } else {
            Some(Entry {
                date,
                time,
                nick,
                msg,
            })
        }
    }
}

pub enum ParseRes<'a> {
    Done,
    Skip,
    Yield(Entry<'a>),
}

impl<R: BufRead> Parser<R> {
    pub fn new(r: R) -> Self {
        Self {
            r,
            buf: String::new(),
        }
    }

    pub fn next_entry(&mut self) -> ParseRes {
        self.buf.clear();
        match self.r.read_line(&mut self.buf) {
            Err(_) => ParseRes::Done,
            Ok(0) => ParseRes::Done,
            Ok(_) => match Entry::from_line(&self.buf.trim()) {
                Some(e) => ParseRes::Yield(e),
                None => ParseRes::Skip,
            },
        }
    }
}

pub fn parse_file(f: &str) -> Fallible<Parser<Box<dyn BufRead>>> {
    let r = Box::new({
        let f = std::fs::File::open(f)?;
        std::io::BufReader::new(f)
    });
    Ok(Parser::new(r))
}
