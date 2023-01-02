use std::env;
use std::fs;
use std::io::{Cursor, Write};
use std::io::BufRead;
use std::io::{Seek, SeekFrom};
use std::process;

use imap;
use mail_parser;
use native_tls;
use serde_derive::Deserialize;
use toml;

#[derive(Deserialize)]
struct Filter {
    all: All,
}

#[derive(Deserialize)]
struct All {
    block: Vec<String>,
    line: Vec<String>,
}

fn fetch_inbox_top() -> imap::error::Result<Option<String>> {
    let imap_host: String = match env::var("IMAP_HOST") {
        Ok(val) => val,
        Err(err) => {
            println!("err: {}", err);
            process::exit(1);
        }
    };

    let imap_user = match env::var("IMAP_USER") {
        Ok(val) => val,
        Err(err) => {
            println!("err: {}", err);
            process::exit(1);
        }
    };

    let imap_password = match env::var("IMAP_PASSWORD") {
        Ok(val) => val,
        Err(err) => {
            println!("err: {}", err);
            process::exit(1);
        }
    };

    let tls = native_tls::TlsConnector::builder().build().unwrap();
    let client = imap::connect((imap_host.clone(), 993), imap_host, &tls).unwrap();
    let mut session = client.login(imap_user, imap_password).map_err(|e| e.0)?;

    session.select("INBOX")?;

    let sequences = session.search("UNSEEN")?;
    let uid = if let Some(l) = sequences.iter().next() {
        l
    } else {
        return Ok(None);
    };

    let messages = session.fetch(format!("{}", uid), "RFC822")?;
    let message = if let Some(m) = messages.iter().next() {
        m
    } else {
        return Ok(None);
    };
    session.store(format!("{}", uid), "-FLAGS (\\Seen)")?;
    let body = message.body().expect("message did not have a body!");
    let body = std::str::from_utf8(body)
        .expect("message was not valid utf-8")
        .to_string();

    // log out from server
    session.logout()?;

    Ok(Some(body))
}

fn main() {
    let mail = fetch_inbox_top().unwrap().unwrap();
    let message = mail_parser::Message::parse(mail.as_bytes()).unwrap();
    let from: String;
    match message.from().clone() {
        mail_parser::HeaderValue::Address(addr) => {
            from = format!("{} <{}>", addr.name.unwrap(), addr.address.unwrap());
        }
        _default => {
            println!("err: invalid from header value");
            process::exit(1)
        }
    }
    let mut body: String = "".to_string();
    for i in message.text_body.clone().into_iter() {
        body = format!("{}", message.body_text(i - 1).unwrap());
    }

    let filter_filename = "./filter.toml";
    let filter_body = match fs::read_to_string(filter_filename) {
        Ok(c) => c,
        Err(_) => {
            println!("err: could not read the file `{}`", filter_filename);
            process::exit(1);
        }
    };
    let filter: Filter = match toml::from_str(&filter_body) {
        Ok(f) => f,
        Err(err) => {
            println!("err: failed to parse the file `{}`: {}", filter_filename, err);
            process::exit(1);
        }
    };

    let mut c = Cursor::new(Vec::new());
    for block in body.split("\n\n") {
        c.write_all(&block.as_bytes()).unwrap();
        c.write_all(b"\n").unwrap();
    }
    c.seek(SeekFrom::Start(0)).unwrap();

    println!("from: {}", from);
    println!("date: {}", message.date().unwrap().to_rfc3339());
    println!("subject: {}", message.subject().unwrap());
    println!("body:");
    for line in c.lines() {
        println!("{}", line.unwrap());
    }
}