use crossterm::{
    QueueableCommand,
    cursor::MoveTo,
    event::{Event, KeyCode, KeyModifiers, poll, read},
    style::{Color, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
};
use rand::{seq::IndexedRandom, thread_rng};
use std::{env, io::Stdout, process::exit, str};
use std::{
    io::{ErrorKind, Read, Write, stdout},
    time::Duration,
};
use std::{net::TcpStream, thread};

struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

fn get_random_color() -> Color {
    let colors = [
        Color::Blue,
        Color::Cyan,
        Color::Green,
        Color::Magenta,
        Color::Red,
        Color::Yellow,
        Color::White,
    ];

    *colors.choose(&mut rand::rng()).unwrap()
}

fn chat_window(stdout: &mut impl Write, chat: &[String], boundary: Rect, color: Color) {
    let n = chat.len();
    let m = n.checked_sub(boundary.h).unwrap_or(0);

    for (dy, line) in chat.iter().skip(m).enumerate() {
        stdout
            .queue(MoveTo(boundary.x as u16, (boundary.y + dy) as u16))
            .unwrap();
        if let Some((name, rest)) = line.split_once(": ") {
            stdout.queue(SetForegroundColor(color)).unwrap();
            stdout.write_all(name.as_bytes()).unwrap();

            stdout.queue(ResetColor).unwrap();
            stdout.write_all(b": ").unwrap();
            stdout.write_all(rest.as_bytes()).unwrap();
        } else {
            stdout.write_all(line.as_bytes()).unwrap();
        }
    }
}

struct Command {
    name: &'static str,
    desc: &'static str,
    run: fn(&mut TcpStream, &str, chat: &mut Vec<String>, nick: &mut String),
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

fn auth_command(stream: &mut TcpStream, token: &str, _chat: &mut Vec<String>, nick: &mut String) {
    stream.write_all(token.as_bytes()).unwrap();
}
fn quit_command(
    _stream: &mut TcpStream,
    _prompt: &str,
    _chat: &mut Vec<String>,
    nick: &mut String,
) {
    exit(1);
}
fn help_command(_stream: &mut TcpStream, _prompt: &str, chat: &mut Vec<String>, nick: &mut String) {
    let mut buf = String::new();
    buf.push_str("Usage: \r\n");
    for cmd in COMMANDS {
        let total = format!("{} - {}\r\n", cmd.name, cmd.desc);
        chat.push(total + "\r\n");
    }
}
fn set_nick_command(
    _stream: &mut TcpStream,
    prompt: &str,
    chat: &mut Vec<String>,
    nick: &mut String,
) {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        chat.push("Nickname cannot be empty.\r\n".to_string());
    } else {
        chat.push(format!("Nickname changed from {} to {}\r\n", nick, trimmed));
        *nick = trimmed.to_string();
    }
}

fn main() {
    let mut args = env::args();
    let _program = args.next().expect("program name");
    let ip = args.next().expect("provide ip");
    let port = args.next().expect("port");

    let mut stream = TcpStream::connect(format!("{ip}:{port}")).unwrap();
    stream.set_nonblocking(true).unwrap();

    let mut stdout = stdout();
    terminal::enable_raw_mode().unwrap();
    let (mut w, mut h) = terminal::size().unwrap();
    let mut bar = "-".repeat(w as usize);
    let mut prompt = String::new();
    let mut quit = false;
    let mut chat = Vec::new();
    let mut nick = String::from("anon");
    let color = get_random_color();

    chat.push("Commands:\r\n".to_string());
    chat.push("/auth <token>\r\n".to_string());
    chat.push("/quit\r\n".to_string());
    chat.push("/help\r\n".to_string());
    chat.push("/nick <name>\r\n".to_string());
    chat.push("\r\n".to_string());
    chat.push("You are offline. Use /auth <token> to authenticate.".to_string());
    let mut buf = [0; 64];

    while !quit {
        while poll(Duration::ZERO).unwrap() {
            match read().unwrap() {
                Event::Resize(nw, nh) => {
                    w = nw;
                    h = nh;
                    bar = "-".repeat(w as usize);
                }
                Event::Paste(data) => {
                    prompt.push_str(&data);
                }
                Event::Key(event) => match event.code {
                    KeyCode::Char(x) => {
                        if x == 'c' && event.modifiers.contains(KeyModifiers::CONTROL) {
                            stream
                                .write_all(format!("{nick} left.").as_bytes())
                                .unwrap();
                            quit = true;
                        } else {
                            prompt.push(x);
                        }
                    }
                    KeyCode::Enter => {
                        let mut is_command = false;
                        for command in COMMANDS.iter() {
                            if prompt.starts_with(command.name) {
                                let token = prompt[command.name.len()..].trim_start();
                                (command.run)(
                                    &mut stream,
                                    if token.is_empty() { "dummy" } else { token },
                                    &mut chat,
                                    &mut nick,
                                );
                                prompt.clear();
                                is_command = true;
                                break;
                            }
                        }
                        if !is_command {
                            let full_msg = format!("{nick}: {prompt}");
                            stream.write_all(full_msg.as_bytes()).unwrap();
                            chat.push(full_msg.clone());
                            prompt.clear();
                        }
                    }
                    KeyCode::Esc => {
                        prompt.clear();
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        match stream.read(&mut buf) {
            Ok(n) => {
                if n > 0 {
                    chat.push(str::from_utf8(&buf[0..n]).unwrap().to_string());
                } else {
                    quit = true;
                }
            }
            Err(err) => {
                if err.kind() != ErrorKind::WouldBlock {
                    panic!("{err}");
                }
            }
        }

        stdout
            .queue(terminal::Clear(terminal::ClearType::All))
            .unwrap();

        chat_window(
            &mut stdout,
            &chat,
            Rect {
                x: 0,
                y: 0,
                w: w as usize,
                h: h as usize - 2,
            },
            color,
        );

        stdout.queue(MoveTo(0, h - 2)).unwrap();
        stdout.write_all(bar.as_bytes()).unwrap();

        stdout.queue(MoveTo(0, h - 1)).unwrap();

        let bytes = prompt.as_bytes();
        stdout
            .write_all(bytes.get(0..w as usize).unwrap_or(bytes))
            .unwrap();

        stdout.flush().unwrap();
        thread::sleep(Duration::from_millis(30));
    }
    terminal::disable_raw_mode().unwrap();
}
