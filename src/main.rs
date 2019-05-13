use {
    irc::client::prelude::*,
    markov::Chain as MChain,
    std::{
        collections::HashMap,
        error::Error,
        ffi::OsStr,
        fs::{self, File},
        path,
        sync::{Arc, Mutex},
        thread, time,
    },
};

#[macro_use]
extern crate serde_derive;

mod log_parse;

/// Temporary storage of chains
pub struct Chains {
    data_dir: path::PathBuf,
    ttl: time::Duration, // number of seconds before discarding
    cached: HashMap<String, CachedChain>,
}

/// Chain cached in memory (with "last used" timestamp for eviction)
pub struct CachedChain {
    last_used: time::Instant,
    chain: Arc<Chain>,
}

/// Chain for a nick
#[derive(Serialize, Deserialize)]
pub struct Chain {
    nick: String,
    c: MChain<String>,
}

/// A generic type of errors
pub type Fallible<T> = Result<T, Box<Error>>;

impl Chain {
    pub fn new(n: &str) -> Self {
        Chain {
            nick: n.to_owned(),
            c: MChain::new(),
        }
    }
}

impl CachedChain {
    pub fn touch(&mut self) {
        self.last_used = time::Instant::now();
    }
    pub fn from_path(nick: &str, p: &path::Path) -> Fallible<Self> {
        let r = std::io::BufReader::new(File::open(p)?);
        let c: MChain<String> = bincode::deserialize_from(r)?;
        Ok(CachedChain {
            last_used: time::Instant::now(),
            chain: Arc::new(Chain {
                nick: nick.into(),
                c,
            }),
        })
    }
}

const DATA_DIR: &'static str = "./data";

fn path_for_nick(data_dir: &path::Path, nick: &str) -> path::PathBuf {
    let mut path = path::PathBuf::new();
    path.push(data_dir);
    path.push(nick);
    path.set_extension("bin");
    path
}

impl Chains {
    pub fn with_path(p: &std::path::Path) -> Self {
        Chains {
            data_dir: p.into(),
            ttl: std::time::Duration::from_secs(20),
            cached: HashMap::new(),
        }
    }
    pub fn new() -> Self {
        Chains::with_path(path::Path::new(DATA_DIR))
    }

    pub fn nicks(&self) -> Fallible<Vec<String>> {
        let mut v = vec![];
        for p in fs::read_dir(&self.data_dir)? {
            let path = match p {
                Ok(x) => x.path(),
                Err(..) => continue,
            };
            if path.extension() == Some(OsStr::new("bin")) {
                v.push(path.file_stem().unwrap().to_string_lossy().to_string())
            }
        }
        Ok(v)
    }

    // cleanup old entries
    fn cleanup(&mut self) {
        let now = time::Instant::now();
        let ttl = self.ttl;
        self.cached.retain(|nick, c| {
            let keep = now - c.last_used <= ttl;
            if !keep {
                println!("cleanup entry for `{}`", nick);
            }
            keep
        });
    }

    /// Find chain for this nickname
    pub fn find_nick(&mut self, nick: &str) -> Option<Arc<Chain>> {
        let mut opt = self.cached.get_mut(nick);
        if let Some(ref mut c) = opt {
            c.touch();
            opt.map(|c| c.chain.clone())
        } else {
            let path = path_for_nick(&self.data_dir, nick);
            let c = CachedChain::from_path(nick, &path).ok();
            if let Some(c) = c {
                self.cached.insert(nick.to_string(), c);
                self.cached.get(nick).map(|c| c.chain.clone())
            } else {
                println!(
                    "could not load chain for nick {:?} (path: {:?})",
                    nick, path
                );
                None
            }
        }
    }
}

fn read_file(s: &str) -> Fallible<HashMap<String, Chain>> {
    let mut parser = log_parse::parse_file(s)?;
    let mut chains = HashMap::new();
    loop {
        match parser.next_entry() {
            log_parse::ParseRes::Skip => (),
            log_parse::ParseRes::Done => break,
            log_parse::ParseRes::Yield(record) => {
                //println!("parsed record {:?}", &record);
                let c = {
                    if !chains.contains_key(&record.nick) {
                        chains.insert(record.nick.to_string(), Chain::new(&record.nick));
                    }
                    chains.get_mut(&record.nick).unwrap()
                };
                c.c.feed_str(record.msg);
            }
        }
    }
    Ok(chains)
}

const PREFIX: &'static str = "!charlie";

fn parse_irc_cmd<'a>(msg: &'a Message) -> Option<&'a str> {
    match msg.command {
        Command::PRIVMSG(ref _tgt, ref line) if line.starts_with(PREFIX) => {
            let rest = &line[PREFIX.len()..].trim();
            Some(rest)
        }
        _ => None,
    }
}

fn serve(data_dir: &path::Path) -> Fallible<()> {
    let chains = Arc::new(Mutex::new(Chains::with_path(data_dir)));
    println!(
        "known nicks: {:?}",
        chains.lock().unwrap().nicks().iter().collect::<Vec<_>>()
    );

    let config = Config {
        nickname: Some("charliebot".to_string()),
        server: Some("chat.freenode.org".to_string()),
        port: Some(7000),
        channels: Some(vec!["#arch-fr-free".to_string()]),
        use_ssl: Some(true),
        ..Config::default()
    };

    let client = IrcClient::from_config(config).map_err(|e| e.to_string())?;
    client.identify().map_err(|e| e.to_string())?;

    // thread to cleanup chains regularly
    let thread = {
        let c = chains.clone();
        thread::spawn(move || loop {
            thread::sleep(time::Duration::from_secs(3));
            c.lock().unwrap().cleanup();
        })
    };

    client
        .for_each_incoming(|message| {
            print!("{}", message);
            if let Some(nick) = parse_irc_cmd(&message) {
                let nick = log_parse::normalize_nick(nick);
                println!(">>> irc command detected for {:?}", &nick);
                if let Some(chain) = chains.lock().unwrap().find_nick(&nick) {
                    let reply_to = {
                        let r = message.response_target();
                        if r.is_none() {
                            return;
                        } else {
                            r.unwrap()
                        }
                    };
                    // try to find a reply of adequate length
                    let reply = chain
                        .c
                        .str_iter()
                        .take(500)
                        .skip_while(|s| s.len() < 20 || s.len() > 100)
                        .next()
                        .unwrap_or_else(|| "oh noes :(".to_string());
                    println!(">>> reply {}", &reply);
                    client.send_privmsg(reply_to, reply).unwrap();
                } else {
                    println!("no chain found for {:?}", nick);
                }
            }
        })
        .map_err(|e| e.to_string())?;

    thread.join().unwrap();
    Ok(())
}

fn generate(data_dir: &path::Path, file: &str) -> Fallible<()> {
    println!("create dir {:?}", data_dir);
    fs::create_dir_all(data_dir)?;
    let chains = read_file(file)?;
    for (nick, chain) in chains.iter() {
        if nick.trim() == "" {
            continue;
        }
        let path = path_for_nick(data_dir, nick);
        //println!("save for nick `{}` in {:?}", nick, path);
        let mut w = std::io::BufWriter::new(File::create(path)?);
        bincode::serialize_into(&mut w, &chain.c)?;
    }
    Ok(())
}

fn main() -> Fallible<()> {
    let args = std::env::args();
    let data_dir = path::Path::new(DATA_DIR);
    match args.collect::<Vec<_>>().as_slice() {
        &[_, ref cmd] if cmd == "help" => {
            println!("commands: help | generate $file | serve");
        }
        &[_, ref cmd, ref file] if cmd == "generate" => {
            generate(data_dir, file)?;
        }
        &[_, ref cmd] if cmd == "serve" => {
            serve(data_dir)?;
        }
        _ => return Err("wrong command".into()),
    }
    Ok(())
}
