use {
    irc::{client::prelude::*},
    markov::Chain,
    std::{collections::HashMap, error::Error, fs::File},
};

#[macro_use]
extern crate serde_derive;

mod log_parse;

/// A chain per nick
#[derive(Serialize, Deserialize)]
pub struct Chains(HashMap<String, Chain<String>>);

/// A generic type of errors
pub type Fallible<T> = Result<T, Box<Error>>;

impl Chains {
    pub fn new() -> Self {
        Chains(HashMap::default())
    }

    pub fn find_nick(&self, nick: &str) -> Option<&Chain<String>> {
        self.0.get(nick)
    }

    pub fn find_nick_mut(&mut self, nick: &str) -> &mut Chain<String> {
        if !self.0.contains_key(nick) {
            self.0.insert(nick.to_string(), Chain::new());
        }
        self.0.get_mut(nick).unwrap()
    }
}

fn read_file(s: &str) -> Fallible<Chains> {
    let mut parser = log_parse::parse_file(s)?;
    let mut chains = Chains::new();
    loop {
        match parser.next_entry() {
            log_parse::ParseRes::Skip => (),
            log_parse::ParseRes::Done => break,
            log_parse::ParseRes::Yield(record) => {
                //println!("parsed record {:?}", &record);
                chains.find_nick_mut(&record.nick).feed_str(record.msg);
            }
        }
    }
    Ok(chains)
}

fn read_gen(file: &str) -> Fallible<()> {
    let chains = read_file(file)?;

    for nick in &["companion_cube", "Polochon_street"] {
        chains
            .find_nick(nick)
            .unwrap()
            .str_iter_for(3)
            .for_each(|s| {
                println!("{}: {}", nick, s);
            });
    }
    Ok(())
}

const PREFIX : &'static str = "!charlie";

fn parse_irc_cmd<'a>(msg: &'a Message) -> Option<&'a str> {
    match msg.command {
        Command::PRIVMSG(ref _tgt, ref line) if line.starts_with(PREFIX) => {
            let rest = &line[PREFIX.len()..].trim();
            Some(rest)
        },
        _ => None,
    }
}

fn serve() -> Fallible<()> {
    let chains: Chains = {
        let r = std::io::BufReader::new(File::open("chains.bin")?);
        bincode::deserialize_from(r)?
    };
    println!("known nicks: {:?}", chains.0.iter().map(|x| x.0).collect::<Vec<_>>());

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

    client.for_each_incoming(|message| {
        print!("{}", message);
        if let Some(nick) = parse_irc_cmd(&message) {            
            let nick = log_parse::normalize_nick(nick);
            println!(">>> irc command detected for {:?}", &nick);
            if let Some(chain) = chains.find_nick(&nick) {
                let reply_to = {
                    let r = message.response_target();
                    if r.is_none() { return } else { r.unwrap() }
                };
                // try to find a reply of adequate length
                let reply =
                    chain.str_iter().take(500)
                    .skip_while(|s| s.len() < 20 || s.len() > 100)
                    .next().unwrap_or_else(|| "oh noes :(".to_string());
                println!(">>> reply {}", &reply);
                client.send_privmsg(reply_to, reply).unwrap();
            }
        }
    }).map_err(|e| e.to_string())?;

    Ok(())
}

fn main() -> Fallible<()> {
    let args = std::env::args();
    match args.collect::<Vec<_>>().as_slice() {
        &[_, ref cmd] if cmd == "help" => {
            println!("commands: help | generate $file | read-gen $file");
        }
        &[_, ref cmd, ref file] if cmd == "read-gen" => {
            read_gen(file)?;
        }
        &[_, ref cmd, ref file] if cmd == "generate" => {
            let chains = read_file(file)?;
            let mut w = std::io::BufWriter::new(File::create("chains.bin")?);
            bincode::serialize_into(&mut w, &chains)?;
        }
        &[_, ref cmd] if cmd == "serve" => {
            serve()?;
        }
        _ => return Err("wrong command".into()),
    }
    Ok(())
}
