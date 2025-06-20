use std::{
    collections::HashMap,
    fmt::{self, Write as OtherWrite},
    io::{Read, Write},
    net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream},
    panic::RefUnwindSafe,
    process::exit,
    result,
    str::from_utf8,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
    thread,
    time::{Duration, SystemTime},
};

use getrandom::fill;

type Result<T> = result::Result<T, ()>;

const SAFE_MODE: bool = false;
const BAN_LIMIT: Duration = Duration::from_secs(10 * 60);
const MESSAGE_RATE: Duration = Duration::from_secs(1);
const STRIKE_LIMIT: i32 = 10;

struct Sens<T>(T);

impl<T: fmt::Display> fmt::Display for Sens<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if SAFE_MODE {
            writeln!(f, "[REDACTED]")
        } else {
            // writeln!(f, "{}", self.0)
            self.0.fmt(f)
        }
    }
}

struct Command {
    name: &'static str,
    desc: &'static str,
    run: fn(stream: Arc<TcpStream>, token: &str, nick: &mut String),
}

const COMMANDS: &[Command] = &[
    Command {
        name: "/auth",
        desc: "Authenticate using a token",
        run: auth_command,
    },
    Command {
        name: "/quit",
        desc: "Quit",
        run: quit_command,
    },
    Command {
        name: "/help",
        desc: "Print this help",
        run: help_command,
    },
    Command {
        name: "/nick",
        desc: "Change your nickname",
        run: set_nick_command,
    },
];

fn auth_command(stream: Arc<TcpStream>, token: &str, _nick: &mut String) {
    stream.as_ref().write_all(token.as_bytes()).unwrap();
}
fn quit_command(_stream: Arc<TcpStream>, _prompt: &str, _nick: &mut String) {
    // let msg = format!("{nick} left.");
    // stream.as_ref().write_all(msg.as_bytes()).unwrap();
    exit(1);
}
fn help_command(stream: Arc<TcpStream>, _prompt: &str, _nick: &mut String) {
    let mut buf = String::new();
    buf.push_str("Usage: \r\n");
    for cmd in COMMANDS {
        let total = format!("{} - {}\r\n", cmd.name, cmd.desc);
        // chat.push(total + "\r\n");
        stream.as_ref().write_all(total.as_bytes()).unwrap();
    }
}
fn set_nick_command(stream: Arc<TcpStream>, prompt: &str, nick: &mut String) {
    let mut trimmed: &str;
    trimmed = prompt.trim();
    if prompt.len() > 16 {
        trimmed = prompt[0..16].trim();
    }
    if trimmed.is_empty() || trimmed == *nick {
        stream
            .as_ref()
            .write_all("Nickname cannot by empty or same.\r\n".as_bytes())
            .unwrap();
    } else {
        stream
            .as_ref()
            .write_all(format!("Nickname changed from {} to {}\r\n", nick, trimmed).as_bytes())
            .unwrap();
        *nick = trimmed.to_string();
    }
}

enum Message {
    ClientConnected { author: Arc<TcpStream> },
    ClientDisconnected { author_addr: SocketAddr },
    New { message_type: NewMessageType },
}

enum NewMessageType {
    TextMessage {
        author_addr: SocketAddr,
        bytes: Vec<u8>,
    },
    CommandMessage {
        author_addr: SocketAddr,
        bytes: Vec<u8>,
    },
}

struct Client {
    conn: Arc<TcpStream>,
    last_message: SystemTime,
    strike_count: i32,
    authed: bool,
}

fn server(messages: Receiver<Message>, token: String) -> Result<()> {
    let mut clients = HashMap::<SocketAddr, Client>::new();
    let mut banned_mfs = HashMap::<IpAddr, SystemTime>::new();
    loop {
        let msg = messages.recv().expect("The server receiver is not hung up");
        match msg {
            Message::ClientConnected { author } => {
                let author_addr = author.peer_addr().expect("TODO: cache the peer addr");
                let mut banned_at = banned_mfs.remove(&author_addr.ip());
                let now = SystemTime::now();

                banned_at = banned_at.and_then(|banned_at| {
                    let diff = now
                        .duration_since(banned_at)
                        .expect("TODO: dont crash if the clock went backwards");

                    if diff >= BAN_LIMIT {
                        None
                    } else {
                        Some(banned_at)
                    }
                });

                if let Some(banned_at) = banned_at {
                    let diff = now
                        .duration_since(banned_at)
                        .expect("TODO: dont crash if the clock went backwards");
                    banned_mfs.insert(author_addr.ip().clone(), banned_at);
                    let secs = (BAN_LIMIT - diff).as_secs_f32();
                    let mut author = author.as_ref();
                    println!(
                        "INFO: Client {author_addr} tried to connect but that mf is banned for {secs}"
                    );
                    let _ = writeln!(author, "You are banned buddy, {secs} secs left",).map_err(
                        |err| eprintln!("ERROR: could not send banned msg to {author_addr}: {err}"),
                    );
                    let _ = author.shutdown(Shutdown::Both).map_err(|err| {
                        eprintln!("ERROR: could not shutdown socket for {author_addr}: {err}")
                    });
                } else {
                    eprintln!("INFO: Client {author_addr} connected");
                    clients.insert(
                        author_addr,
                        Client {
                            conn: author.clone(),
                            last_message: now,
                            strike_count: 0,
                            authed: false,
                        },
                    );
                    // let _ = write!(
                    //     author.as_ref(),
                    //     "Commands: \r\n/auth <token>\r\n/quit\r\n/help"
                    // )
                    // .map_err(|err| {
                    //     eprintln!(
                    //         "ERROR: could not send Token prompt to {}: {}",
                    //         Sens(author_addr),
                    //         Sens(err)
                    //     );
                    // });
                }
            }
            Message::ClientDisconnected { author_addr } => {
                eprintln!("INFO: Client {author_addr} disconnected");
                clients.remove(&author_addr);
            }
            Message::New { message_type } => {
                match message_type {
                    NewMessageType::TextMessage { author_addr, bytes } => {
                        if let Some(author) = clients.get_mut(&author_addr) {
                            let now = SystemTime::now();
                            let diff = now
                                .duration_since(author.last_message)
                                .expect("TODO: dont crash if the clock went backwards");
                            if diff >= MESSAGE_RATE {
                                if let Ok(text) = from_utf8(&bytes) {
                                    if author.authed {
                                        // broadcasting
                                        println!("INFO: Client {author_addr} sent {bytes:?}");
                                        for (addr, client) in clients.iter() {
                                            if author_addr != *addr && client.authed {
                                                let _ = client.conn.as_ref().write(&bytes).map_err(|err| {
                                        eprintln!("ERROR: could not broadcast message to all the clients from {author_addr}: {err}")
                                    });
                                            }
                                        }
                                    } else {
                                        if text == token {
                                            author.authed = true;
                                            println!("INFO: {} authorized!", Sens(author_addr));
                                            let _ = writeln!(author.conn.as_ref(), "Welcome!").map_err(|err| {
                                        eprintln!(
                                            "ERROR: could not send welcome prompt to client {}: {}",
                                            Sens(author_addr),
                                            Sens(err)
                                        );
                                    });
                                        } else {
                                            let _ = writeln!(author.conn.as_ref(), "Invalid token!").map_err(|err| {
                                            eprintln!(
                                                "ERROR: could not notify client {} about invalid token: {}",
                                                Sens(author_addr),
                                                Sens(err)
                                            );
                                        });
                                            let _ = author.conn.shutdown(Shutdown::Both).map_err(
                                                |err| {
                                                    eprintln!(
                                                        "ERROR: could not shutdown {}: {}",
                                                        Sens(author_addr),
                                                        Sens(err)
                                                    );
                                                },
                                            );
                                            clients.remove(&author_addr);
                                        }
                                    }
                                } else {
                                    author.strike_count += 1;
                                    if author.strike_count >= STRIKE_LIMIT {
                                        println!("INFO: Client {author_addr} got banned");
                                        banned_mfs.insert(author_addr.ip().clone(), now);
                                        let _ = writeln!(author.conn.as_ref(), "You are banned!").map_err(|err| {
                                        eprintln!("ERROR: could not send banned message to {author_addr}: {err}")
                                    });
                                        let _ = author.conn.shutdown(Shutdown::Both).map_err(|err| {
                                    eprintln!(
                                        "ERROR: could not shutdown socket for {author_addr}: {err}"
                                    )
                                });
                                    }
                                }
                            } else {
                                author.strike_count += 1;
                                if author.strike_count >= STRIKE_LIMIT {
                                    println!("INFO: Client {author_addr} got banned");
                                    banned_mfs.insert(author_addr.ip().clone(), now);
                                    let _ = writeln!(author.conn.as_ref(), "You are banned!").map_err(|err| {
                                        eprintln!("ERROR: could not send banned message to {author_addr}: {err}")
                                    });
                                    let _ = author.conn.shutdown(Shutdown::Both).map_err(|err| {
                                eprintln!(
                                    "ERROR: could not shutdown socket for {author_addr}: {err}"
                                )
                            });
                                }
                            }
                        }
                    }
                    NewMessageType::CommandMessage { author_addr, bytes } => {
                        if let Some(author) = clients.get_mut(&author_addr) {
                            let now = SystemTime::now();
                            let diff = now
                                .duration_since(author.last_message)
                                .expect("TODO: dont crash if the clock went backwards");
                            if diff >= MESSAGE_RATE {
                                if let Ok(text) = from_utf8(&bytes) {
                                    // send command to self
                                    let mut is_command = false;
                                    let mut nick = String::from("dummy");
                                    for command in COMMANDS {
                                        if text.starts_with(command.name) {
                                            let user_token =
                                                text[command.name.len()..].trim_start();
                                            if command.name == "/auth" && !author.authed {
                                                if token == user_token {
                                                    author.authed = true;
                                                    println!(
                                                        "INFO: {} authorized!",
                                                        Sens(author_addr)
                                                    );
                                                    let _ = writeln!(author.conn.as_ref(), "Welcome!").map_err(|err| {
                                        eprintln!(
                                            "ERROR: could not send welcome prompt to client {}: {}",
                                            Sens(author_addr),
                                            Sens(err)
                                        );
                                    });
                                                } else {
                                                    let _ = writeln!(author.conn.as_ref(), "Invalid token!").map_err(|err| {
                                            eprintln!(
                                                "ERROR: could not notify client {} about invalid token: {}",
                                                Sens(author_addr),
                                                Sens(err)
                                            );
                                        });
                                                    let _ = author
                                                        .conn
                                                        .shutdown(Shutdown::Both)
                                                        .map_err(|err| {
                                                            eprintln!(
                                                                "ERROR: could not shutdown {}: {}",
                                                                Sens(author_addr),
                                                                Sens(err)
                                                            );
                                                        });
                                                    clients.remove(&author_addr);
                                                }
                                            } else {
                                                author
                                                    .conn
                                                    .as_ref()
                                                    .write_all("Already authorized.".as_bytes())
                                                    .unwrap();
                                                break;
                                            }
                                            (command.run)(
                                                author.conn.clone(),
                                                if user_token.is_empty() {
                                                    ""
                                                } else {
                                                    user_token
                                                },
                                                &mut nick,
                                            );
                                            is_command = true;
                                            break;
                                        }
                                    }
                                    if !is_command {
                                        println!("INFO: Client {author_addr} sent {bytes:?}");
                                        for (addr, client) in clients.iter() {
                                            if author_addr != *addr && client.authed {
                                                let _ = client.conn.as_ref().write(&bytes).map_err(|err| {
                                        eprintln!("ERROR: could not broadcast message to all the clients from {author_addr}: {err}")
                                    });
                                            }
                                        }
                                    }
                                } else {
                                    author.strike_count += 1;
                                    if author.strike_count >= STRIKE_LIMIT {
                                        println!("INFO: Client {author_addr} got banned");
                                        banned_mfs.insert(author_addr.ip().clone(), now);
                                        let _ = writeln!(author.conn.as_ref(), "You are banned!").map_err(|err| {
                                        eprintln!("ERROR: could not send banned message to {author_addr}: {err}")
                                    });
                                        let _ = author.conn.shutdown(Shutdown::Both).map_err(|err| {
                                    eprintln!(
                                        "ERROR: could not shutdown socket for {author_addr}: {err}"
                                    )
                                });
                                    }
                                }
                            } else {
                                author.strike_count += 1;
                                if author.strike_count >= STRIKE_LIMIT {
                                    println!("INFO: Client {author_addr} got banned");
                                    banned_mfs.insert(author_addr.ip().clone(), now);
                                    let _ = writeln!(author.conn.as_ref(), "You are banned!").map_err(|err| {
                                        eprintln!("ERROR: could not send banned message to {author_addr}: {err}")
                                    });
                                    let _ = author.conn.shutdown(Shutdown::Both).map_err(|err| {
                                eprintln!(
                                    "ERROR: could not shutdown socket for {author_addr}: {err}"
                                )
                            });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn client(stream: Arc<TcpStream>, messages: Sender<Message>) -> Result<()> {
    let author_addr = stream.peer_addr().map_err(|err| {
        eprintln!("ERROR: could not get peer address: {err}");
    })?;

    messages
        .send(Message::ClientConnected {
            author: stream.clone(),
        })
        .map_err(|err| eprintln!("ERROR: could not send message to the server thread: {err}"))?;

    let mut buffer = vec![0; 64];
    loop {
        let n = stream.as_ref().read(&mut buffer).map_err(|err| {
            eprintln!("ERROR: could not read msg from client: {err}");
            let _ = messages
                .send(Message::ClientDisconnected { author_addr })
                .map_err(|err| {
                    eprintln!("ERROR: could not send message that client disconnected: {err}")
                });
        })?;
        if n > 0 {
            let mut bytes = Vec::new();
            for x in &buffer[0..n] {
                if *x >= 32 {
                    bytes.push(*x);
                }
            }
            let slash = std::str::from_utf8(&bytes[0..1]).map_err(|e| {
                eprintln!("Invalid UTF-8: {e}");
                // propagate or handle the error
            })?;
            if slash == "/" {
                messages
                    .send(Message::New {
                        message_type: NewMessageType::CommandMessage { author_addr, bytes },
                    })
                    .map_err(|err| {
                        eprintln!("ERROR: could not send message to the server thread: {err}");
                    })?;
            } else {
                messages
                    .send(Message::New {
                        message_type: NewMessageType::TextMessage { author_addr, bytes },
                    })
                    .map_err(|err| {
                        eprintln!("ERROR: could not send message to the server thread: {err}");
                    })?;
            }
        } else {
            let _ = messages
                .send(Message::ClientDisconnected { author_addr })
                .map_err(|err| {
                    eprintln!("ERROR: could not send message that client disconnected: {err}")
                });
            break;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let mut buffer = [0; 16];
    let _ = fill(&mut buffer).map_err(|err| {
        eprintln!("ERROR: could not generate random access token: {err}");
    });

    let mut token = String::new();
    for x in buffer.iter() {
        let _ = write!(&mut token, "{x:02X}").map_err(|err| {
            eprintln!("ERROR: could not write token bytes to buffer: {err}");
        });
    }
    println!("Token: {token}");

    let addr = "0.0.0.0:6969";
    println!("INFO: Listening to {}", Sens(addr));
    let listener = TcpListener::bind(addr)
        .map_err(|err| eprintln!("ERROR: could not bind to {}: {}", Sens(addr), Sens(err)))?;
    let (message_sender, message_receiver) = channel();
    thread::spawn(|| server(message_receiver, token));

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let message_sender = message_sender.clone();
                thread::spawn(|| client(stream.into(), message_sender));
            }
            Err(err) => {
                eprintln!("ERROR: could not accept connection: {}", Sens(err));
            }
        }
    }
    Ok(())
}
