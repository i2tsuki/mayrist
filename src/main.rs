use std::env;
use std::process;

use imap;
use mail_parser;
use native_tls;

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
    let body = fetch_inbox_top().unwrap().unwrap();
    let message = mail_parser::Message::parse(body.as_bytes()).unwrap();
    let from: String;
    match message.from().clone() {
        mail_parser::HeaderValue::Address(addr) => {
            from = format!("{} <{}>", addr.name.unwrap(), addr.address.unwrap());
        }
        _default => process::exit(1)
    }
    println!("from: {}", from);
    println!("date: {}", message.date().unwrap().to_rfc3339());
    println!("subject: {}", message.subject().unwrap());
    println!("body:");
    for i in message.text_body.clone().into_iter() {
        println!("{}", message.body_text(i - 1).unwrap());
    }
}
