use clap::Parser;
use clap_derive::Parser;

use std::env;
use std::fs;
use std::io::{BufRead, Cursor, Seek, SeekFrom, Write};
use std::process;

use imap;
use mail_parser;
use native_tls;
use regex::Regex;
use serde_derive::Deserialize;
use toml;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// From e-mail address to filter body based on it
    #[arg(long, default_value = "")]
    from: String,
    /// File that includes body to filter
    #[arg(long, default_value = "", requires = "from")]
    input: String,
    /// Flag that indicates whether delete or not after fetching the email
    #[arg(long, default_value = "false")]
    delete: bool,
}

#[derive(Deserialize)]
struct Filter {
    search: Vec<Search>,
    all: All,
    message: Vec<Message>,
}

#[derive(Deserialize)]
struct Search {
    from: String,
}

#[derive(Deserialize)]
struct All {
    block: Vec<String>,
    line: Vec<String>,
}

#[derive(Deserialize)]
struct Message {
    from: String,
    block: Vec<String>,
}

fn fetch_inbox_top(
    host: String,
    user: String,
    password: String,
    search_from: String,
) -> imap::error::Result<(u32, Option<String>)> {
    let tls = native_tls::TlsConnector::builder().build().unwrap();
    let client = imap::connect((host.clone(), 993), host, &tls).unwrap();
    let mut session = client.login(user, password).map_err(|e| e.0)?;

    session.select("INBOX")?;

    eprintln!("SEARCH query: {}", search_from);
    let sequences = session.search(format!("{}", search_from))?;
    let uid = if let Some(l) = sequences.iter().next() {
        l
    } else {
        return Ok((0, None));
    };

    let messages = session.fetch(format!("{}", uid), "RFC822")?;
    let message = if let Some(m) = messages.iter().next() {
        m
    } else {
        return Ok((0, None));
    };
    // session.store(format!("{}", uid), "-FLAGS (\\Seen)")?;
    // session.store(format!("{}", uid), "+FLAGS (\\Deleted)")?;
    let body = message.body().expect("message did not have a body!");
    let body = std::str::from_utf8(body)
        .expect("message was not valid utf-8")
        .to_string();

    // log out from server
    session.logout()?;

    Ok((*uid, Some(body)))
}

fn main() {
    let filter_filename = "./filter.toml";
    let filter_body = match fs::read_to_string(filter_filename) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("err: could not read the file `{}`", filter_filename);
            process::exit(1);
        }
    };
    let filter: Filter = match toml::from_str(&filter_body) {
        Ok(f) => f,
        Err(err) => {
            eprintln!(
                "err: failed to parse the file `{}`: {}",
                filter_filename, err
            );
            process::exit(1);
        }
    };

    let args = Args::parse();
    if args.from != "" && args.input != "" {
        // Select filter to match from
        let mut filter_message = &Message {
            from: "".to_string(),
            block: Vec::new(),
        };
        for f in &filter.message {
            if args.from.contains(&f.from) {
                filter_message = f;
            }
        }
        
        let mut c = Cursor::new(Vec::new());
        let body = match fs::read_to_string(args.input) {
            Ok(c) => c,
            Err(_) => {
                eprintln!("err: could not read the file `{}`", filter_filename);
                process::exit(1);
            }
        };    
        for block in body.replace("\r", "").split("\n\n") {
            let mut filter_match: bool = false;
            if !filter_match {
                for filter_block in &filter_message.block {
                    let re =
                        Regex::new(&(filter_block.trim().replace("<url>", r"http[s]*://\S+") + "$"))
                            .unwrap();
                    if block.trim() == filter_block.trim() || re.is_match(block.trim()) {
                        filter_match = true;
                        break;
                    }
                }
            }
            if !filter_match {
                c.write_all(&block.as_bytes()).unwrap();
                c.write_all(b"\n\n").unwrap();
            }
        }
        c.seek(SeekFrom::Start(0)).unwrap();
        println!("body:");
        let mut previous_is_blank = false;
        for l in c.clone().lines() {
            let line: String = l.unwrap();
            if !previous_is_blank && line == "" {
                previous_is_blank = true;
                println!("");
            } else if line == "" {
                print!("");
            } else {
                previous_is_blank = false;
                println!("{}", line);
            }
        }
    } else {
        let imap_host: String = match env::var("IMAP_HOST") {
            Ok(val) => val,
            Err(err) => {
                eprintln!("err: {}", err);
                process::exit(1);
            }
        };
        let imap_user = match env::var("IMAP_USER") {
            Ok(val) => val,
            Err(err) => {
                eprintln!("err: {}", err);
                process::exit(1);
            }
        };
        let imap_password = match env::var("IMAP_PASSWORD") {
            Ok(val) => val,
            Err(err) => {
                eprintln!("err: {}", err);
                process::exit(1);
            }
        };

        let mut search_from = "".to_string();
        let mut iter = filter.search.iter();
        if filter.search.len() > 0 {
            search_from = format!("FROM {} UNSEEN", filter.search[0].from);
            iter.next();
        }
        for search in iter {
            search_from += format!(" OR FROM {} UNSEEN", search.from).as_str();
        }
        let (mid, mail) = match fetch_inbox_top(imap_host.clone(), imap_user.clone(), imap_password.clone(), search_from) {
            Ok((0, None)) => {
                eprintln!("there are no messages in the mailbox.");
                process::exit(0);
            }
            Ok((id, m)) => (id, m.unwrap()),
            Err(err) => {
                eprintln!("err: failed to get the message: {}", err);
                process::exit(1)
            }
        };
        let message = mail_parser::Message::parse(mail.as_bytes()).unwrap();

        let from: String;
        match message.from().clone() {
            mail_parser::HeaderValue::Address(addr) => {
                let name: &str = match addr.name {
                    Some(ref name) => &name,
                    None => "",
                };
                from = format!("{} <{}>", name, addr.address.unwrap());
            }
            _default => {
                eprintln!("err: invalid from header value");
                process::exit(1)
            }
        }
        let mut body: String = "".to_string();
        // eprintln!("{:?}", message.body_text(0));
        for i in message.text_body.clone().into_iter() {
            if i > 0 {
                body = format!("{}", message.body_text(i - 1).unwrap());
            } else {
                body = format!("{}", message.body_text(i).unwrap());
            }
        }

        // remove blocks with block filter rule defined in `filter.yaml`
        let mut c = Cursor::new(Vec::new());
        for block in body.replace("\r", "").split("\n\n") {
            let mut filter_match: bool = false;
            for filter_block in &filter.all.block {
                let re = Regex::new(&(filter_block.trim().replace("<url>", r"http[s]*://\S+") + "$"))
                    .unwrap();
                if block.trim() == filter_block.trim() || re.is_match(block.trim()) {
                    filter_match = true;
                    break;
                }
            }
            if !filter_match {
                let mut filter_message = &Message {
                    from: "".to_string(),
                    block: Vec::new(),
                };
                for f in &filter.message {
                    if from.contains(&f.from) {
                        filter_message = f;
                    }
                }
                for filter_block in &filter_message.block {
                    let re =
                        Regex::new(&(filter_block.trim().replace("<url>", r"http[s]*://\S+") + "$"))
                            .unwrap();
                    if block.trim() == filter_block.trim() || re.is_match(block.trim()) {
                        filter_match = true;
                        break;
                    }
                }
            }
            if !filter_match {
                c.write_all(&block.as_bytes()).unwrap();
                c.write_all(b"\n\n").unwrap();
            }
        }
        c.seek(SeekFrom::Start(0)).unwrap();

        // remove lines with line filter rule defined in `filter.yaml`
        let mut str: String = "".to_string();
        for l in c.clone().lines() {
            let line: String = l.unwrap();
            let mut filter_match: bool = false;
            for filter_line in &filter.all.line {
                if line == *filter_line {
                    filter_match = true;
                    break;
                }
            }
            if !filter_match {
                str = format!("{}{}\n", str, line);
            }
        }
        c = Cursor::new(Vec::new());
        c.write_all(str.as_bytes()).unwrap();
        c.seek(SeekFrom::Start(0)).unwrap();
        
        eprintln!("original_body: \n{}", body);
        println!("from: {}", from);
        println!("date: {}", message.date().unwrap().to_rfc3339());
        println!("subject: {}", message.subject().unwrap());
        println!("body:");
        let mut previous_is_blank = false;
        for l in c.clone().lines() {
            let line: String = l.unwrap();
            if !previous_is_blank && line == "" {
                previous_is_blank = true;
                println!("");
            } else if line == "" {
                print!("");
            } else {
                previous_is_blank = false;
                println!("{}", line);
            }
        }
        if args.delete {
            let tls = native_tls::TlsConnector::builder().build().unwrap();
            let client = imap::connect((imap_host.clone(), 993), imap_host, &tls).unwrap();
            let mut session = client.login(imap_user, imap_password).map_err(|e| e.0).unwrap();
            session.select("INBOX").unwrap();
            session.store(format!("{}", mid), "+FLAGS (\\Deleted)").unwrap();
            eprint!("Deleted the message: {}", mid);
        }    
    }
}
