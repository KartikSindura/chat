use std::{
    collections::HashMap,
    fmt::{self, Write as OtherWrite},
    io::{Read, Write},
    net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream},
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

enum Message {
    ClientConnected {
        author: Arc<TcpStream>,
    },
    ClientDisconnected {
        author_addr: SocketAddr,
    },
    New {
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
                    let _ = write!(author.as_ref(), "Token: ").map_err(|err| {
                        eprintln!(
                            "ERROR: could not send Token prompt to {}: {}",
                            Sens(author_addr),
                            Sens(err)
                        );
                    });
                }
            }
            Message::ClientDisconnected { author_addr } => {
                eprintln!("INFO: Client {author_addr} disconnected");
                clients.remove(&author_addr);
            }
            Message::New { author_addr, bytes } => {
                if let Some(author) = clients.get_mut(&author_addr) {
                    let now = SystemTime::now();
                    let diff = now
                        .duration_since(author.last_message)
                        .expect("TODO: dont crash if the clock went backwards");
                    if diff >= MESSAGE_RATE {
                        if let Ok(text) = from_utf8(&bytes) {
                            println!("INFO: Client {author_addr} sent {bytes:?}");

                            if author.authed {
                                // broadcasting
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
                                    let _ = author.conn.shutdown(Shutdown::Both).map_err(|err| {
                                        eprintln!(
                                            "ERROR: could not shutdown {}: {}",
                                            Sens(author_addr),
                                            Sens(err)
                                        );
                                    });
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
            messages
                .send(Message::New { author_addr, bytes })
                .map_err(|err| {
                    eprintln!("ERROR: could not send message to the server thread: {err}");
                })?;
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
